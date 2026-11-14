package network.hone.app.fw

import android.annotation.SuppressLint
import android.app.PendingIntent
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbDeviceConnection
import android.hardware.usb.UsbEndpoint
import android.hardware.usb.UsbManager
import android.os.Build
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeout
import java.util.UUID

/** Flipper on USB: CDC-ACM serial (VID 0x0483, PID 0x5740). The CLI starts the RPC session. */
class UsbFlipper private constructor(private val conn: UsbDeviceConnection, private val outEp: UsbEndpoint,
                                     private val inEp: UsbEndpoint, private val ifaces: List<android.hardware.usb.UsbInterface>) : FlipperTransport {
    override val name = "USB"
    override val chunk = 8192
    override val incoming = Channel<ByteArray>(Channel.UNLIMITED)
    @Volatile private var open = true
    private val reader = Thread({
        val b = ByteArray(16384)
        while (open) {
            val n = conn.bulkTransfer(inEp, b, b.size, 200)
            if (n > 0) incoming.trySend(b.copyOf(n))
        }
    }, "flipper-usb").apply { isDaemon = true; start() }

    override suspend fun send(b: ByteArray) {
        var off = 0
        while (off < b.size) {
            val n = conn.bulkTransfer(outEp, b.copyOfRange(off, minOf(b.size, off + 16384)), minOf(16384, b.size - off), 5_000)
            if (n <= 0) error("USB write failed"); off += n
        }
    }

    override fun close() { open = false; ifaces.forEach { runCatching { conn.releaseInterface(it) } }; conn.close() }

    companion object {
        const val VID = 0x0483; const val PID = 0x5740
        private const val ACTION = "network.hone.app.USB_PERMISSION"

        fun find(ctx: Context): UsbDevice? = ctx.getSystemService(UsbManager::class.java)?.deviceList?.values
            ?.firstOrNull { it.vendorId == VID && it.productId == PID }

        /** Asks for USB permission (system dialog) if needed. */
        suspend fun permission(ctx: Context, d: UsbDevice): Boolean {
            val um = ctx.getSystemService(UsbManager::class.java) ?: return false
            if (um.hasPermission(d)) return true
            val r = CompletableDeferred<Boolean>()
            val rx = object : BroadcastReceiver() {
                override fun onReceive(c: Context, i: Intent) { r.complete(i.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)) }
            }
            if (Build.VERSION.SDK_INT >= 33) ctx.registerReceiver(rx, IntentFilter(ACTION), Context.RECEIVER_NOT_EXPORTED)
            else @Suppress("UnspecifiedRegisterReceiverFlag") ctx.registerReceiver(rx, IntentFilter(ACTION))
            try {
                um.requestPermission(d, PendingIntent.getBroadcast(ctx, 0, Intent(ACTION).setPackage(ctx.packageName),
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
                return withTimeout(120_000) { r.await() }
            } finally { runCatching { ctx.unregisterReceiver(rx) } }
        }

        /** Opens the CDC data interface, raises DTR/RTS and starts the RPC session on the CLI. */
        suspend fun open(ctx: Context, d: UsbDevice): UsbFlipper {
            val um = ctx.getSystemService(UsbManager::class.java) ?: error("no USB host")
            val conn = um.openDevice(d) ?: error("cannot open USB device")
            val comm = (0 until d.interfaceCount).map { d.getInterface(it) }.firstOrNull { it.interfaceClass == UsbConstants.USB_CLASS_COMM }
            val data = (0 until d.interfaceCount).map { d.getInterface(it) }.first { it.interfaceClass == UsbConstants.USB_CLASS_CDC_DATA }
            val claimed = listOfNotNull(comm, data).onEach { conn.claimInterface(it, true) }
            val eps = (0 until data.endpointCount).map { data.getEndpoint(it) }.filter { it.type == UsbConstants.USB_ENDPOINT_XFER_BULK }
            val out = eps.first { it.direction == UsbConstants.USB_DIR_OUT }; val inn = eps.first { it.direction == UsbConstants.USB_DIR_IN }
            // SET_CONTROL_LINE_STATE: DTR | RTS
            conn.controlTransfer(0x21, 0x22, 0x03, comm?.id ?: 0, null, 0, 1000)
            val t = UsbFlipper(conn, out, inn, claimed)
            // The CLI answers with a prompt; switch it to RPC and drop what the CLI printed.
            delay(300); while (t.incoming.tryReceive().isSuccess) { }
            t.send("start_rpc_session\r".toByteArray())
            var seen = ""
            withTimeout(5_000) { while (!seen.contains("start_rpc_session")) seen += String(t.incoming.receive()) }
            delay(200); while (t.incoming.tryReceive().isSuccess) { }
            return t
        }
    }
}

/** Flipper over Bluetooth LE: the serial service (needs a one-time pairing / bond). */
@SuppressLint("MissingPermission")
class BleFlipper private constructor(private val gatt: BluetoothGatt, private val cb: Cb) : FlipperTransport {
    override val name = "Bluetooth"
    override val chunk get() = (cb.mtu - 3).coerceIn(20, 512)
    override val incoming get() = cb.incoming
    private val lock = Mutex()

    override suspend fun send(b: ByteArray) = lock.withLock {
        var off = 0
        while (off < b.size) {
            val room = cb.credit.coerceAtMost(chunk)
            if (room <= 0) { delay(20); continue }
            val part = b.copyOfRange(off, minOf(b.size, off + room))
            cb.writeDone = CompletableDeferred()
            val rx = cb.rx ?: error("serial characteristic missing")
            if (Build.VERSION.SDK_INT >= 33) gatt.writeCharacteristic(rx, part, BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT)
            else { @Suppress("DEPRECATION") run { rx.value = part; rx.writeType = BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT; gatt.writeCharacteristic(rx) } }
            if (!withTimeout(10_000) { cb.writeDone!!.await() }) error("Bluetooth write failed")
            cb.credit -= part.size; off += part.size
        }
    }

    override fun close() { runCatching { gatt.disconnect(); gatt.close() } }

    class Cb : BluetoothGattCallback() {
        val incoming = Channel<ByteArray>(Channel.UNLIMITED)
        val ready = CompletableDeferred<Boolean>()
        @Volatile var mtu = 23
        @Volatile var credit = 0
        @Volatile var rx: BluetoothGattCharacteristic? = null
        @Volatile var writeDone: CompletableDeferred<Boolean>? = null
        private var descQueue = ArrayDeque<BluetoothGattDescriptor>()

        override fun onConnectionStateChange(g: BluetoothGatt, status: Int, state: Int) {
            if (state == BluetoothProfile.STATE_CONNECTED) g.requestMtu(512)
            else { ready.complete(false); writeDone?.complete(false) }
        }
        override fun onMtuChanged(g: BluetoothGatt, m: Int, status: Int) { mtu = m; g.discoverServices() }
        override fun onServicesDiscovered(g: BluetoothGatt, status: Int) {
            val s = g.getService(SERVICE) ?: run { ready.complete(false); return }
            rx = s.getCharacteristic(RX)
            val tx = s.getCharacteristic(TX); val fc = s.getCharacteristic(FLOW)
            for (c in listOfNotNull(tx, fc)) {
                g.setCharacteristicNotification(c, true)
                c.getDescriptor(CCCD)?.let { d -> descQueue.add(d.also { @Suppress("DEPRECATION") run {
                    it.value = if (c == tx) BluetoothGattDescriptor.ENABLE_INDICATION_VALUE else BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE } }) }
            }
            nextDesc(g)
        }
        private fun nextDesc(g: BluetoothGatt) {
            val d = descQueue.removeFirstOrNull()
            if (d == null) { g.getService(SERVICE)?.getCharacteristic(FLOW)?.let { g.readCharacteristic(it) } ?: ready.complete(true); return }
            @Suppress("DEPRECATION") g.writeDescriptor(d)
        }
        override fun onDescriptorWrite(g: BluetoothGatt, d: BluetoothGattDescriptor, status: Int) = nextDesc(g)
        @Deprecated("API < 33") override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            @Suppress("DEPRECATION") onValue(c, c.value ?: ByteArray(0)); ready.complete(true)
        }
        override fun onCharacteristicRead(g: BluetoothGatt, c: BluetoothGattCharacteristic, v: ByteArray, status: Int) { onValue(c, v); ready.complete(true) }
        @Deprecated("API < 33") override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic) {
            @Suppress("DEPRECATION") onValue(c, c.value ?: ByteArray(0))
        }
        override fun onCharacteristicChanged(g: BluetoothGatt, c: BluetoothGattCharacteristic, v: ByteArray) = onValue(c, v)
        override fun onCharacteristicWrite(g: BluetoothGatt, c: BluetoothGattCharacteristic, status: Int) {
            writeDone?.complete(status == BluetoothGatt.GATT_SUCCESS)
        }
        private fun onValue(c: BluetoothGattCharacteristic, v: ByteArray) {
            when (c.uuid) {
                TX -> incoming.trySend(v)
                FLOW -> if (v.size >= 4) credit = ((v[0].toInt() and 0xFF) shl 24) or ((v[1].toInt() and 0xFF) shl 16) or
                    ((v[2].toInt() and 0xFF) shl 8) or (v[3].toInt() and 0xFF)
            }
        }
    }

    companion object {
        val SERVICE: UUID = UUID.fromString("8fe5b3d5-2e7f-4a98-2a48-7acc60fe0000")
        val TX: UUID = UUID.fromString("19ed82ae-ed21-4c9d-4145-228e61fe0000")    // Flipper -> phone (indicate)
        val RX: UUID = UUID.fromString("19ed82ae-ed21-4c9d-4145-228e62fe0000")    // phone -> Flipper (write)
        val FLOW: UUID = UUID.fromString("19ed82ae-ed21-4c9d-4145-228e63fe0000")  // free buffer, 4 bytes big-endian
        val CCCD: UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")

        /** Bonded Flippers (paired once in the phone's Bluetooth settings). */
        fun bonded(ctx: Context): List<BluetoothDevice> = runCatching {
            ctx.getSystemService(BluetoothManager::class.java)?.adapter?.bondedDevices?.filter { (it.name ?: "").startsWith("Flipper") }
        }.getOrNull().orEmpty()

        suspend fun open(ctx: Context, d: BluetoothDevice): BleFlipper {
            val cb = Cb()
            val g = d.connectGatt(ctx, false, cb, BluetoothDevice.TRANSPORT_LE)
            if (!withTimeout(20_000) { cb.ready.await() }) { runCatching { g.close() }; error("Flipper serial service not found; is Bluetooth on and paired?") }
            return BleFlipper(g, cb)
        }
    }
}

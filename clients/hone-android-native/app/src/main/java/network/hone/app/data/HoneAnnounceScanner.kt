package network.hone.app.data

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothManager
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.bouncycastle.crypto.params.Ed25519PublicKeyParameters
import org.bouncycastle.crypto.signers.Ed25519Signer
import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** A HONE device heard over BLE (e.g. the Flipper), from its signed announce. */
data class HoneDevice(
    val deviceId: String,          // first 8 bytes of the pubkey, hex
    val pubkey: String,
    val owner: String,             // as claimed in the announce
    val signatureOk: Boolean,
    val chainOwnerOk: Boolean?,    // null = not checked yet / no node reachable
    val batteryPct: Int,
    val caps: List<String>,
    val extPower: Boolean,
    val paused: Boolean,
    val counter: Long,
    val newestReading: Long,
    val rssi: Int,
    val lastSeenMs: Long,
)

/**
 * Listens for HONE announces: BLE manufacturer data, company 0xFFFF, "HA", seq,
 * idx<<4|total, then up to 23 bytes of a signed v2 frame (type 8). Fragments with
 * the same (address, seq) are joined, the ed25519 signature is checked against the
 * pubkey inside the body, and the claimed owner is checked against the chain: the
 * device's sensors must be enrolled to that owner.
 */
object HoneAnnounceScanner {
    private const val TAG = "HoneAnnounce"
    private const val COMPANY = 0xFFFF
    private val CAPS = listOf("subghz", "nfc", "rfid", "ibutton", "ir", "wifi")

    private val _devices = MutableStateFlow<Map<String, HoneDevice>>(emptyMap())
    val devices: StateFlow<Map<String, HoneDevice>> = _devices.asStateFlow()

    private val frags = HashMap<String, HashMap<Int, ByteArray>>()
    private val done = LinkedHashSet<String>()
    private val chainChecked = HashMap<String, Pair<Boolean?, Long>>() // deviceId -> (ok, at)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    @Volatile private var running = false

    fun hasPermission(ctx: Context): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S)
            ctx.checkSelfPermission(Manifest.permission.BLUETOOTH_SCAN) == PackageManager.PERMISSION_GRANTED
        else
            ctx.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

    @SuppressLint("MissingPermission")
    @Synchronized
    fun start(ctx: Context) {
        if (running || !hasPermission(ctx)) return
        val scanner = ctx.getSystemService(BluetoothManager::class.java)?.adapter?.bluetoothLeScanner
        if (scanner == null) { Log.w(TAG, "no BLE scanner (Bluetooth off?)"); return }
        val filter = ScanFilter.Builder()
            .setManufacturerData(COMPANY, byteArrayOf('H'.code.toByte(), 'A'.code.toByte()),
                byteArrayOf(0xFF.toByte(), 0xFF.toByte()))
            .build()
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_BALANCED)
            .build()
        runCatching { scanner.startScan(listOf(filter), settings, callback) }
            .onSuccess { running = true; Log.i(TAG, "scanning for HONE announces") }
            .onFailure { Log.e(TAG, "startScan failed", it) }
    }

    private val callback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) {
            val md = result.scanRecord?.getManufacturerSpecificData(COMPANY) ?: return
            if (md.size < 5 || md[0] != 'H'.code.toByte() || md[1] != 'A'.code.toByte()) return
            val seq = md[2].toInt() and 0xFF
            val idx = (md[3].toInt() and 0xFF) shr 4
            val total = md[3].toInt() and 0x0F
            if (total == 0 || idx >= total) return
            val key = "${result.device.address}/$seq"
            synchronized(this@HoneAnnounceScanner) {
                if (key in done) { touch(result); return }
                val m = frags.getOrPut(key) { HashMap() }
                m[idx] = md.copyOfRange(4, md.size)
                if (m.size < total) return
                done.add(key); frags.remove(key)
                while (done.size > 256) done.remove(done.first())
                if (frags.size > 256) frags.clear()
                val frame = (0 until total).fold(ByteArray(0)) { acc, i -> acc + m.getValue(i) }
                parse(frame, result.rssi)?.let {
                    addrToId[result.device.address] = it.deviceId
                    publish(it)
                }
            }
        }
        override fun onScanFailed(errorCode: Int) {
            Log.e(TAG, "scan failed: $errorCode"); running = false
        }
    }

    private val addrToId = HashMap<String, String>()

    /** Refresh rssi / last-seen on every repeat fragment of an already-verified announce. */
    private fun touch(r: ScanResult) {
        val id = addrToId[r.device.address] ?: return
        val d = _devices.value[id] ?: return
        _devices.value = _devices.value + (id to d.copy(rssi = r.rssi, lastSeenMs = System.currentTimeMillis()))
    }

    private fun parse(f: ByteArray, rssi: Int): HoneDevice? {
        if (f.size < 32 + 64 || f[0] != 'H'.code.toByte() || f[1] != '2'.code.toByte()) return null
        val type = f[2].toInt() and 0xFF
        val blen = f[3].toInt() and 0xFF
        if (type != 8 || f.size != 32 + blen + 64 || blen < 56) return null
        val bb = ByteBuffer.wrap(f).order(ByteOrder.LITTLE_ENDIAN)
        val devId = f.copyOfRange(4, 12)
        val counter = bb.getInt(12).toLong() and 0xFFFFFFFFL
        val body = f.copyOfRange(32, 32 + blen)
        val sig = f.copyOfRange(32 + blen, f.size)
        val pk = body.copyOfRange(0, 32)
        val b = ByteBuffer.wrap(body).order(ByteOrder.LITTLE_ENDIAN)
        val caps = b.getShort(32).toInt() and 0xFFFF
        val newest = b.getInt(34).toLong() and 0xFFFFFFFFL
        val batt = body[38].toInt() and 0xFF
        val flags = body[39].toInt() and 0xFF
        val owner = String(body.copyOfRange(40, 56), Charsets.UTF_8).trimEnd('\u0000').trim()
        val sigOk = runCatching {
            Ed25519Signer().run {
                init(false, Ed25519PublicKeyParameters(pk, 0))
                update(f, 0, 32 + blen)
                verifySignature(sig)
            }
        }.getOrDefault(false) && devId.contentEquals(pk.copyOfRange(0, 8))
        val id = devId.hex()
        return HoneDevice(
            deviceId = id, pubkey = pk.hex(), owner = owner, signatureOk = sigOk,
            chainOwnerOk = chainChecked[id]?.first,
            batteryPct = batt, caps = CAPS.filterIndexed { i, _ -> (caps shr i) and 1 == 1 },
            extPower = flags and 1 != 0, paused = flags and 2 != 0,
            counter = counter, newestReading = newest, rssi = rssi,
            lastSeenMs = System.currentTimeMillis(),
        )
    }

    private fun publish(d: HoneDevice) {
        _devices.value = _devices.value + (d.deviceId to d)
        Log.i(TAG, "announce ${d.deviceId} owner=${d.owner} sig=${d.signatureOk} batt=${d.batteryPct}%")
        val last = chainChecked[d.deviceId]
        if (d.signatureOk && (last == null || System.currentTimeMillis() - last.second > 300_000)) {
            chainChecked[d.deviceId] = (last?.first) to System.currentTimeMillis()
            scope.launch {
                val ok = chainOwner(d)
                chainChecked[d.deviceId] = ok to System.currentTimeMillis()
                _devices.value[d.deviceId]?.let {
                    _devices.value = _devices.value + (d.deviceId to it.copy(chainOwnerOk = ok))
                }
            }
        }
    }

    /** The owner claim holds if one of the device's sensors is enrolled on chain to that
     *  owner (and, when the node reports it, to this exact device key). */
    private fun chainOwner(d: HoneDevice): Boolean? {
        var reached = false
        for (suffix in listOf("subghz", "tags", "ir", "wifi")) {
            val (code, body) = ChainHttp.getWithCode("/api/sensor/flipper-${d.deviceId}-$suffix") ?: continue
            reached = true
            if (code == 404 || body == null) continue
            val j = runCatching { JSONObject(body) }.getOrNull() ?: continue
            val owner = j.optString("owner")
            val devKey = j.optString("device_pubkey", "")
            if (owner == d.owner && (devKey.isEmpty() || devKey.equals(d.pubkey, ignoreCase = true))) return true
            if (owner.isNotEmpty() && owner != d.owner) return false
        }
        return if (reached) false else null
    }

    private fun ByteArray.hex(): String = joinToString("") { "%02x".format(it) }
}

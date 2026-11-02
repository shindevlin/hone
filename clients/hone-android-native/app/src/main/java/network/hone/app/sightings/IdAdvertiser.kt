package network.hone.app.sightings

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothManager
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertisingSet
import android.bluetooth.le.AdvertisingSetCallback
import android.bluetooth.le.AdvertisingSetParameters
import android.bluetooth.le.BluetoothLeAdvertiser
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.SystemClock
import android.util.Log

/**
 * Advertises this phone's per-window public key pk_w (spec §1a, Find My style).
 * Manufacturer data, company 0xFFFF, non-connectable, non-scannable:
 *
 *   'h' 'K' ver=0x02 win8 frag tag[2] payload
 *     frag = idx<<4 | total
 *     extended advert (BLE 5, when supported): total=1, payload = pk_w[32]   (39 bytes)
 *     legacy fallback: two adverts, total=2, payload = pk_w[0..16] / pk_w[16..32] (23 bytes each)
 *   tag = HKDF(share_k, "hone-tag"||window)[0..2]: observers join the two legacy fragments by
 *   (win8, tag), because each advertising set has its own random address.
 *
 * Address rotation: apps cannot set the address; every new AdvertisingSet gets a fresh
 * random one. At each UTC quarter-hour all sets are stopped and new ones started, so the
 * key and the address(es) change together.
 */
object IdAdvertiser {
    private const val TAG = "HoneSight"
    const val COMPANY = 0xFFFF
    val PREFIX = byteArrayOf('h'.code.toByte(), 'K'.code.toByte())
    const val VERSION: Byte = 2

    @Volatile var advertising = false; private set
    @Volatile var lastError = ""; private set
    @Volatile var mode = ""; private set
    private val sets = ArrayList<AdvertisingSetCallback>()
    private var currentWindow = Long.MIN_VALUE
    private var startingAt = 0L

    fun hasPermission(ctx: Context): Boolean = Build.VERSION.SDK_INT < Build.VERSION_CODES.S ||
        ctx.checkSelfPermission(Manifest.permission.BLUETOOTH_ADVERTISE) == PackageManager.PERMISSION_GRANTED

    fun frames(k: IdKeys.WindowKey, extended: Boolean): List<ByteArray> {
        val head = PREFIX + byteArrayOf(VERSION, (k.window and 0xFF).toByte())
        return if (extended) listOf(head + byteArrayOf(0x01) + k.tag + k.pk)
        else listOf(head + byteArrayOf(0x02) + k.tag + k.pk.copyOfRange(0, 16),
                    head + byteArrayOf(0x12) + k.tag + k.pk.copyOfRange(16, 32))
    }

    @SuppressLint("MissingPermission")
    @Synchronized
    fun ensure(ctx: Context, key: IdKeys.WindowKey) {
        val now = SystemClock.elapsedRealtime()
        if (key.window == currentWindow && (advertising || now - startingAt < 10_000)) return
        if (!hasPermission(ctx)) { lastError = "no BLUETOOTH_ADVERTISE permission"; return }
        val ad = ctx.getSystemService(BluetoothManager::class.java)?.adapter
        val adv = ad?.bluetoothLeAdvertiser
        if (adv == null) { lastError = "no BLE advertiser (Bluetooth off?)"; advertising = false; return }
        stopAll(adv)
        currentWindow = key.window; startingAt = now
        val ext = ad.isLeExtendedAdvertisingSupported && ad.leMaximumAdvertisingDataLength >= 43
        mode = if (ext) "extended" else "legacy x2"
        frames(key, ext).forEach { start(adv, it, key.window, ext) }
    }

    @SuppressLint("MissingPermission")
    private fun start(adv: BluetoothLeAdvertiser, frame: ByteArray, window: Long, ext: Boolean) {
        val p = AdvertisingSetParameters.Builder()
            .setLegacyMode(!ext).setConnectable(false).setScannable(false)
            .setInterval(AdvertisingSetParameters.INTERVAL_HIGH)          // ~1 s
            .setTxPowerLevel(AdvertisingSetParameters.TX_POWER_MEDIUM)
        if (ext) p.setPrimaryPhy(android.bluetooth.BluetoothDevice.PHY_LE_1M)
            .setSecondaryPhy(android.bluetooth.BluetoothDevice.PHY_LE_1M)
        val data = AdvertiseData.Builder().addManufacturerData(COMPANY, frame)
            .setIncludeDeviceName(false).setIncludeTxPowerLevel(false).build()
        val cb = object : AdvertisingSetCallback() {
            override fun onAdvertisingSetStarted(set: AdvertisingSet?, txPower: Int, status: Int) {
                synchronized(this@IdAdvertiser) {
                    if (status == ADVERTISE_SUCCESS) {
                        advertising = true; lastError = ""
                        Log.i(TAG, "id advert started w=$window mode=$mode frag=${"%02x".format(frame[4])} tx=$txPower")
                    } else {
                        Log.w(TAG, "id advert start failed status=$status mode=$mode")
                        sets.remove(this); startingAt = 0L; lastError = "advertise failed ($status)"
                    }
                }
            }
        }
        sets.add(cb)
        runCatching { adv.startAdvertisingSet(p.build(), data, null, null, null, cb) }
            .onFailure { lastError = it.message ?: "advertise error"; Log.e(TAG, "startAdvertisingSet", it) }
    }

    @SuppressLint("MissingPermission")
    private fun stopAll(adv: BluetoothLeAdvertiser?) {
        sets.forEach { cb -> runCatching { adv?.stopAdvertisingSet(cb) } }
        sets.clear(); advertising = false
    }

    @Synchronized
    fun stop(ctx: Context) {
        stopAll(ctx.getSystemService(BluetoothManager::class.java)?.adapter?.bluetoothLeAdvertiser)
        currentWindow = Long.MIN_VALUE
    }
}

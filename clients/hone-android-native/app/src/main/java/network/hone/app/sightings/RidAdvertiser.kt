package network.hone.app.sightings

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothManager
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertisingSet
import android.bluetooth.le.AdvertisingSetCallback
import android.bluetooth.le.AdvertisingSetParameters
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log

/**
 * Advertises this phone's rid (spec §1a), legacy non-connectable, manufacturer data
 * company 0xFFFF:
 *
 *   'h' 'R'  ver=0x01  win8  rid[16]          (20 bytes; 24 bytes on air)
 *
 * Lower-case 'h' keeps it apart from the Flipper's 'H?' frames. win8 = window & 0xFF lets
 * an owner match a sighting whose observer clock is a little off.
 *
 * Address rotation: Android apps cannot set the advertising address. A new
 * AdvertisingSet gets a freshly generated random address, so at every UTC quarter-hour
 * the old set is stopped and a new one started: rid and address change together.
 * (Verified with an external scanner, see docs/SIGHTINGS.md.) Interval ~1 s, medium power.
 */
object RidAdvertiser {
    private const val TAG = "HoneSight"
    const val COMPANY = 0xFFFF
    val PREFIX = byteArrayOf('h'.code.toByte(), 'R'.code.toByte())
    const val VERSION: Byte = 1

    @Volatile var advertising = false; private set
    @Volatile var lastError = ""; private set
    private var current: AdvertisingSetCallback? = null
    private var currentWindow = Long.MIN_VALUE
    private var startingAt = 0L

    fun hasPermission(ctx: Context): Boolean = Build.VERSION.SDK_INT < Build.VERSION_CODES.S ||
        ctx.checkSelfPermission(Manifest.permission.BLUETOOTH_ADVERTISE) == PackageManager.PERMISSION_GRANTED

    fun payload(rid: ByteArray, window: Long): ByteArray =
        PREFIX + byteArrayOf(VERSION, (window and 0xFF).toByte()) + rid

    /** Start or rotate so the advert carries the rid of [window]. Idempotent per window. */
    @SuppressLint("MissingPermission")
    @Synchronized
    fun ensure(ctx: Context, window: Long) {
        val now = android.os.SystemClock.elapsedRealtime()
        // Same window: nothing to do while advertising or while a start is in flight.
        if (window == currentWindow && (advertising || now - startingAt < 10_000)) return
        if (!hasPermission(ctx)) { lastError = "no BLUETOOTH_ADVERTISE permission"; return }
        val adv = ctx.getSystemService(BluetoothManager::class.java)?.adapter?.bluetoothLeAdvertiser
        if (adv == null) { lastError = "no BLE advertiser (Bluetooth off?)"; advertising = false; return }
        current?.let { runCatching { adv.stopAdvertisingSet(it) } }
        current = null
        advertising = false
        val rid = Rid.rid(Rid.seed(ctx), window)
        val data = AdvertiseData.Builder()
            .addManufacturerData(COMPANY, payload(rid, window))
            .setIncludeDeviceName(false).setIncludeTxPowerLevel(false)
            .build()
        currentWindow = window
        startingAt = now
        start(adv, data, rid, window)
    }

    @SuppressLint("MissingPermission")
    private fun start(adv: android.bluetooth.le.BluetoothLeAdvertiser, data: AdvertiseData,
                      rid: ByteArray, window: Long) {
        val p = AdvertisingSetParameters.Builder()
            .setLegacyMode(true).setConnectable(false).setScannable(false)
            .setInterval(AdvertisingSetParameters.INTERVAL_HIGH)          // ~1 s
            .setTxPowerLevel(AdvertisingSetParameters.TX_POWER_MEDIUM)
        // The own-address type (public / random / non-resolvable) is a system API: apps get
        // the platform default, a random address generated per advertising set.
        val cb = object : AdvertisingSetCallback() {
            override fun onAdvertisingSetStarted(set: AdvertisingSet?, txPower: Int, status: Int) {
                if (status == ADVERTISE_SUCCESS) {
                    advertising = true; lastError = ""
                    Log.i(TAG, "rid advert started w=$window rid=${rid.copyOf(4).hex()}.. tx=$txPower")
                } else {
                    Log.w(TAG, "rid advert start failed status=$status")
                    synchronized(this@RidAdvertiser) {
                        if (current === this) { current = null; startingAt = 0L }
                        lastError = "advertise failed ($status)"
                    }
                }
            }
            override fun onAdvertisingSetStopped(set: AdvertisingSet?) {}
        }
        current = cb
        runCatching { adv.startAdvertisingSet(p.build(), data, null, null, null, cb) }
            .onFailure { lastError = it.message ?: "advertise error"; Log.e(TAG, "startAdvertisingSet", it) }
    }

    @SuppressLint("MissingPermission")
    @Synchronized
    fun stop(ctx: Context) {
        val adv = ctx.getSystemService(BluetoothManager::class.java)?.adapter?.bluetoothLeAdvertiser
        current?.let { cb -> runCatching { adv?.stopAdvertisingSet(cb) } }
        current = null; advertising = false; currentWindow = Long.MIN_VALUE
    }
}

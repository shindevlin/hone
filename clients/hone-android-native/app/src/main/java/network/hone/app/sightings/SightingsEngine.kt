package network.hone.app.sightings

import android.Manifest
import android.annotation.SuppressLint
import android.app.AlarmManager
import android.app.PendingIntent
import android.bluetooth.BluetoothManager
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.SecureRandom
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/** What the Sensors screen shows for the current 15-min window. */
data class LiveCoverage(
    val running: Boolean = false,
    val window: Long = 0,
    val windowEndsMs: Long = 0,
    val cell: String? = null,               // res 9, public
    val locAccM: Float? = null,
    val locAgeS: Long? = null,
    val apCount: Int = 0,
    val apByBand: Map<String, Int> = emptyMap(),
    val bandsSupported: List<String> = emptyList(),
    val bleCrowd: Int = 0,
    val honeDevices: Int = 0,
    val mobileAps: Int = 0,
    val wifiScansOk: Int = 0,
    val wifiScansRefused: Int = 0,
    val wifiThrottled: Boolean? = null,
    val ridPrefix: String = "",
    val advertising: Boolean = false,
    val issues: List<String> = emptyList(),
    val summariesStored: Int = 0,
    val summariesQueued: Int = 0,
    val routerPending: String? = null,
)

/**
 * Phone as a standalone sensor (spec §1b, §2a, phone side of §2c).
 *
 * Per UTC 15-min window it accumulates, per H3 res-9 cell the phone was in:
 *   - Wi-Fi APs as (oid, rssi, channel, band), oid = HMAC(K_day, bssid)[0..8];
 *   - BLE crowd: distinct advertisers as oids;
 *   - HONE rid sightings of other devices (rid, cell, window, rssi);
 *   - Flipper frames heard (stamped with this phone's location);
 *   - mobile-AP events: the same oid seen in two cells on the same UTC day.
 * K_day is random per UTC day and exists only in RAM; no MAC/BSSID/SSID is stored or
 * leaves the device. At the window boundary the per-cell summaries go to the local store
 * and are queued for the (not yet built) chain summary commit.
 */
object SightingsEngine {
    private const val TAG = "HoneSight"
    private const val BLE_CROWD_EVERY_MS = 60_000L
    private const val BLE_CROWD_SCAN_MS = 8_000L
    private const val WIFI_UNTHROTTLED_MS = 120_000L      // throttling disabled in Developer options
    private const val WIFI_FOREGROUND_MS = 30_000L         // app on screen: 4 scans / 2 min allowed
    private const val WIFI_BACKGROUND_MS = 30 * 60_000L    // background: 1 scan / 30 min allowed
    private const val LOC_MAX_AGE_MS = 5 * 60_000L

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var jobs = listOf<Job>()
    @Volatile private var appCtx: Context? = null
    private var store: SightingStore? = null

    private val _live = MutableStateFlow(LiveCoverage())
    val live: StateFlow<LiveCoverage> = _live.asStateFlow()

    /** Set by the Sensors screen while it is visible (Wi-Fi foreground scan allowance). */
    @Volatile var uiVisible = false

    // ── RAM-only day key ──────────────────────────────────────────────────────────────
    private var kDay: ByteArray? = null
    private var kDayIndex = Long.MIN_VALUE
    private val mobileCells = HashMap<String, MutableSet<String>>() // oid -> cells today (RAM only)

    private fun dayKey(nowMs: Long): ByteArray {
        val day = Math.floorDiv(nowMs, 86_400_000L)
        if (day != kDayIndex || kDay == null) {
            kDay = ByteArray(32).also { SecureRandom().nextBytes(it) }
            kDayIndex = day
            mobileCells.clear()
        }
        return kDay!!
    }

    private fun hmac8(key: ByteArray, data: ByteArray): String {
        val m = Mac.getInstance("HmacSHA256"); m.init(SecretKeySpec(key, "HmacSHA256"))
        return m.doFinal(data).copyOf(8).hex()
    }

    private fun macBytes(mac: String): ByteArray =
        runCatching { mac.split(":").map { it.toInt(16).toByte() }.toByteArray() }
            .getOrDefault(mac.lowercase().toByteArray())

    // Persistent private key for the private layer (owner's routers). Not K_day: the owner's
    // own routers must stay recognisable across days. Never leaves the device.
    private var kPriv: ByteArray? = null
    private fun privKey(ctx: Context): ByteArray {
        kPriv?.let { return it }
        val f = File(ctx.filesDir, "hone-node/priv_layer_key")
        val k = if (f.exists() && f.length() == 32L) f.readBytes()
            else ByteArray(32).also { SecureRandom().nextBytes(it); f.parentFile?.mkdirs(); f.writeBytes(it) }
        kPriv = k; return k
    }

    // ── Location ─────────────────────────────────────────────────────────────────────
    @Volatile private var loc: Location? = null
    private val locListener = LocationListener { l -> onLocation(l) }

    private fun onLocation(l: Location) {
        val cur = loc
        // Prefer the newer fix unless it is much worse and the old one is still fresh.
        if (cur == null || l.time > cur.time + 30_000 || l.accuracy <= cur.accuracy) { loc = l; stampCache = null }
    }

    private fun freshLoc(): Location? = loc?.takeIf {
        val age = (SystemClock.elapsedRealtimeNanos() - it.elapsedRealtimeNanos) / 1_000_000
        age in 0..LOC_MAX_AGE_MS
    }

    private data class Stamp(val cell9: String?, val cellf: String?, val acc: Float?)

    @Volatile private var stampCache: Pair<Long, Stamp>? = null

    /** Cells for the current fix (cached 5 s: BLE callbacks arrive many times a second). */
    private fun stamp(): Stamp {
        val now = SystemClock.elapsedRealtime()
        stampCache?.let { (t, st) -> if (now - t < 5_000) return st }
        val l = freshLoc()
        val st = if (l == null) Stamp(null, null, null) else Stamp(
            Cells.cell(l.latitude, l.longitude, Cells.PUBLIC_RES),
            Cells.cell(l.latitude, l.longitude, Cells.PRIVATE_RES), l.accuracy)
        stampCache = now to st
        return st
    }

    fun hasLocation(ctx: Context) =
        ctx.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

    fun hasBackgroundLocation(ctx: Context) = Build.VERSION.SDK_INT < 29 ||
        ctx.checkSelfPermission(Manifest.permission.ACCESS_BACKGROUND_LOCATION) == PackageManager.PERMISSION_GRANTED

    @SuppressLint("MissingPermission")
    private fun startLocation(ctx: Context) {
        if (!hasLocation(ctx)) return
        val lm = ctx.getSystemService(LocationManager::class.java) ?: return
        val providers = lm.allProviders
        val main = if (Build.VERSION.SDK_INT >= 31 && LocationManager.FUSED_PROVIDER in providers)
            LocationManager.FUSED_PROVIDER else if (LocationManager.NETWORK_PROVIDER in providers)
            LocationManager.NETWORK_PROVIDER else LocationManager.GPS_PROVIDER
        runCatching {
            lm.getLastKnownLocation(main)?.let { onLocation(it) }
            // Fused at balanced power, 60 s / 10 m. PASSIVE picks up any GPS fix another
            // component (e.g. the GNSS sensor channel) already pays for.
            lm.requestLocationUpdates(main, 60_000L, 10f, locListener, Looper.getMainLooper())
            lm.requestLocationUpdates(LocationManager.PASSIVE_PROVIDER, 10_000L, 0f, locListener, Looper.getMainLooper())
            Log.i(TAG, "location via $main (+passive)")
        }.onFailure { Log.w(TAG, "location updates failed: ${it.message}") }
    }

    // ── Per-window accumulation ──────────────────────────────────────────────────────
    private class Ap(var rssi: Int, val channel: Int, val band: String)
    private class Acc {
        val aps = HashMap<String, Ap>()
        val ble = HashSet<String>()
        val rids = HashSet<String>()
        val flippers = HashSet<String>()
        val mobile = HashSet<String>()
        var wifiScans = 0
        var bleScanMs = 0L
        var accMin: Float? = null
    }
    private var window = Long.MIN_VALUE
    private val accs = HashMap<String, Acc>()            // cell9 ("" = no fix) -> acc
    private val ridBest = HashMap<String, Int>()          // rid/cellf -> best rssi stored this window
    private val flipperLast = HashMap<String, Long>()     // dev/kind -> last stored ms
    private var wifiOk = 0; private var wifiRefused = 0
    @Volatile private var routerCode: Pair<String, Long>? = null

    private fun acc(s: Stamp): Acc = accs.getOrPut(s.cell9 ?: "") { Acc() }.also { a ->
        s.acc?.let { if (a.accMin == null || it < a.accMin!!) a.accMin = it }
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────────────
    @Synchronized
    fun start(ctx: Context) {
        if (appCtx != null) return
        val c = ctx.applicationContext
        appCtx = c
        store = SightingStore(c)
        Rid.seed(c)
        rollWindow(c, System.currentTimeMillis())
        startLocation(c)
        startRidScan(c)
        registerWifi(c)
        jobs = listOf(
            scope.launch { while (isActive) { tick(); delay(5_000) } },
            scope.launch { while (isActive) { crowdScan(c); delay(BLE_CROWD_EVERY_MS - BLE_CROWD_SCAN_MS) } },
            scope.launch { wifiLoop(c) },
        )
        Log.i(TAG, "sightings engine started; h3=${Cells.available()}")
    }

    @SuppressLint("MissingPermission")
    @Synchronized
    fun stop() {
        val c = appCtx ?: return
        jobs.forEach { it.cancel() }; jobs = emptyList()
        finalizeWindow()
        RidAdvertiser.stop(c)
        runCatching { c.getSystemService(LocationManager::class.java)?.removeUpdates(locListener) }
        runCatching { c.unregisterReceiver(wifiReceiver) }
        runCatching { bleScanner(c)?.stopScan(ridCallback) }
        runCatching { bleScanner(c)?.stopScan(crowdCallback) }
        cancelAlarm(c)
        appCtx = null
        _live.value = _live.value.copy(running = false, advertising = false)
    }

    /** Called every 5 s and by the boundary alarm. Rotates at UTC quarter-hours. */
    fun tick() {
        val c = appCtx ?: return
        rollWindow(c, System.currentTimeMillis())
        // Re-assert the advert (Bluetooth toggled, permission granted later, ...).
        RidAdvertiser.ensure(c, window)
        publishLive(c)
    }

    @Synchronized
    private fun rollWindow(c: Context, nowMs: Long) {
        val w = Rid.window(nowMs)
        if (w == window) return
        if (window != Long.MIN_VALUE) finalizeWindow()
        window = w
        accs.clear(); ridBest.clear()
        dayKey(nowMs)
        RidAdvertiser.ensure(c, w)                 // new rid + new advertising set (address)
        scheduleAlarm(c, Rid.windowStartMs(w + 1))
        store?.prune(w)
        Log.i(TAG, "window $w")
    }

    @Synchronized
    private fun finalizeWindow() {
        val st = store ?: return
        if (window == Long.MIN_VALUE) return
        for ((cell, a) in accs) {
            if (a.aps.isEmpty() && a.ble.isEmpty() && a.rids.isEmpty() && a.flippers.isEmpty()) continue
            val located = cell.isNotEmpty()
            runCatching { st.putSummary(if (located) cell else "unlocated", window, summaryJson(cell, a).toString(), located) }
                .onFailure { Log.e(TAG, "store summary", it) }
        }
        val (n, q) = runCatching { st.summaryCounts() }.getOrDefault(0 to 0)
        Log.i(TAG, "window $window closed: ${accs.size} cell(s); summaries stored=$n queued=$q")
    }

    private fun bandsSupported(c: Context): List<String> {
        val wm = c.getSystemService(WifiManager::class.java) ?: return emptyList()
        return buildList {
            add("2g")
            if (wm.is5GHzBandSupported) add("5g")
            if (Build.VERSION.SDK_INT >= 30 && wm.is6GHzBandSupported) add("6g")
        }
    }

    /** Public summary for one (cell, window). Schema: docs/SIGHTINGS.md. No identities. */
    private fun summaryJson(cell: String, a: Acc): JSONObject {
        val byBand = a.aps.values.groupingBy { it.band }.eachCount()
        val hist = IntArray(7) // <-90, -90..-81, -80..-71, -70..-61, -60..-51, -50..-41, >=-40
        a.aps.values.forEach { hist[rssiBucket(it.rssi)]++ }
        val rssis = a.aps.values.map { it.rssi }.sorted()
        val chans = JSONObject()
        a.aps.values.groupingBy { "${it.band}:${it.channel}" }.eachCount().toSortedMap().forEach { (k, v) -> chans.put(k, v) }
        return JSONObject()
            .put("v", 1).put("cell", cell).put("res", Cells.PUBLIC_RES).put("window", window)
            .put("wifi", JSONObject()
                .put("ap_count", a.aps.size)
                .put("by_band", JSONObject().apply { listOf("2g", "5g", "6g").forEach { put(it, byBand[it] ?: 0) } })
                .put("bands_supported", JSONArray(appCtx?.let { bandsSupported(it) } ?: emptyList<String>()))
                .put("rssi_hist", JSONArray(hist.toList()))
                .put("rssi_p50", if (rssis.isEmpty()) JSONObject.NULL else rssis[rssis.size / 2])
                .put("rssi_max", rssis.lastOrNull() ?: JSONObject.NULL)
                .put("channels", chans)
                .put("scans", a.wifiScans))
            .put("ble", JSONObject().put("crowd", a.ble.size).put("scan_s", a.bleScanMs / 1000))
            .put("hone", JSONObject().put("devices", a.rids.size + a.flippers.size))
            .put("mobile_ap", a.mobile.size)
            .put("loc_acc_m", a.accMin?.let { Math.round(it) } ?: JSONObject.NULL)
    }

    private fun rssiBucket(r: Int): Int = when {
        r < -90 -> 0; r <= -81 -> 1; r <= -71 -> 2; r <= -61 -> 3; r <= -51 -> 4; r <= -41 -> 5; else -> 6
    }

    private fun publishLive(c: Context) {
        val s = stamp()
        val l = loc
        val issues = buildList {
            if (!hasLocation(c)) add("location denied: readings kept unlocated, nothing queued")
            else if (!hasBackgroundLocation(c)) add("location only while app open (allow 'all the time')")
            if (!Cells.available()) add("H3 unavailable")
            if (!RidAdvertiser.hasPermission(c)) add("Bluetooth advertise permission missing")
            else if (RidAdvertiser.lastError.isNotEmpty()) add(RidAdvertiser.lastError)
        }
        val (n, q) = runCatching { store?.summaryCounts() }.getOrNull() ?: (0 to 0)
        val snap = synchronized(this) {
            val a = accs[s.cell9 ?: ""]
            val all = accs.values
            LiveCoverage(
                running = true, window = window, windowEndsMs = Rid.windowStartMs(window + 1),
                cell = s.cell9, locAccM = l?.accuracy,
                locAgeS = l?.let { (SystemClock.elapsedRealtimeNanos() - it.elapsedRealtimeNanos) / 1_000_000_000 },
                apCount = a?.aps?.size ?: 0,
                apByBand = a?.aps?.values?.groupingBy { it.band }?.eachCount() ?: emptyMap(),
                bandsSupported = bandsSupported(c),
                bleCrowd = a?.ble?.size ?: 0,
                honeDevices = all.flatMap { it.rids + it.flippers }.toSet().size,
                mobileAps = a?.mobile?.size ?: 0,
                wifiScansOk = wifiOk, wifiScansRefused = wifiRefused,
                wifiThrottled = throttled(c),
                ridPrefix = Rid.rid(Rid.seed(c), window).copyOf(4).hex(),
                advertising = RidAdvertiser.advertising,
                issues = issues, summariesStored = n, summariesQueued = q,
                routerPending = routerCode?.takeIf { System.currentTimeMillis() < it.second }?.first,
            )
        }
        _live.value = snap
    }

    // ── Boundary alarm (wakes the CPU so rid + address rotate on time in Doze) ────────
    private fun alarmIntent(c: Context) = PendingIntent.getBroadcast(
        c, 7301, Intent(c, WindowAlarmReceiver::class.java),
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )

    private fun scheduleAlarm(c: Context, atMs: Long) {
        val am = c.getSystemService(AlarmManager::class.java) ?: return
        val pi = alarmIntent(c)
        runCatching {
            if (Build.VERSION.SDK_INT < 31 || am.canScheduleExactAlarms())
                am.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, atMs + 250, pi)
            else am.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, atMs + 250, pi)
        }.onFailure { Log.w(TAG, "boundary alarm: ${it.message}") }
    }

    private fun cancelAlarm(c: Context) { runCatching { c.getSystemService(AlarmManager::class.java)?.cancel(alarmIntent(c)) } }

    // ── BLE ──────────────────────────────────────────────────────────────────────────
    private fun bleScanner(c: Context) = c.getSystemService(BluetoothManager::class.java)?.adapter?.bluetoothLeScanner

    private fun hasScan(c: Context) = if (Build.VERSION.SDK_INT >= 31)
        c.checkSelfPermission(Manifest.permission.BLUETOOTH_SCAN) == PackageManager.PERMISSION_GRANTED
    else hasLocation(c)

    /** HONE rids: hardware-filtered, low power, always on (works with the screen off). */
    @SuppressLint("MissingPermission")
    private fun startRidScan(c: Context) {
        if (!hasScan(c)) return
        val f = ScanFilter.Builder().setManufacturerData(RidAdvertiser.COMPANY,
            RidAdvertiser.PREFIX, byteArrayOf(0xFF.toByte(), 0xFF.toByte())).build()
        val s = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_POWER).build()
        runCatching { bleScanner(c)?.startScan(listOf(f), s, ridCallback) }
            .onFailure { Log.w(TAG, "rid scan: ${it.message}") }
    }

    private val ridCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, r: ScanResult) {
            val md = r.scanRecord?.getManufacturerSpecificData(RidAdvertiser.COMPANY) ?: return
            if (md.size != 20 || md[0] != RidAdvertiser.PREFIX[0] || md[1] != RidAdvertiser.PREFIX[1] || md[2] != RidAdvertiser.VERSION) return
            onRid(md.copyOfRange(4, 20).hex(), r.rssi)
        }
        override fun onScanFailed(errorCode: Int) { Log.w(TAG, "rid scan failed $errorCode") }
    }

    private fun onRid(rid: String, rssi: Int) {
        val s = stamp()
        val key = "$rid/${s.cellf}"
        val write: Boolean
        val w: Long
        synchronized(this) {
            acc(s).rids.add(rid)
            w = window
            val best = ridBest[key]
            write = best == null || rssi >= best + 3
            if (write) ridBest[key] = maxOf(rssi, best ?: rssi)
        }
        if (write) {
            runCatching { store?.putRidSighting(rid, w, s.cell9, s.cellf, rssi, System.currentTimeMillis()) }
            Log.i(TAG, "rid sighting ${rid.take(8)}.. rssi=$rssi")
        }
    }

    /** BLE crowd: unfiltered scan, 8 s of every 60 s. (Android withholds unfiltered scan
     *  results while the screen is off, so the crowd count is a screen-on/docked metric.) */
    @SuppressLint("MissingPermission")
    private suspend fun crowdScan(c: Context) {
        if (!hasScan(c)) { delay(BLE_CROWD_SCAN_MS); return }
        val sc = bleScanner(c) ?: run { delay(BLE_CROWD_SCAN_MS); return }
        val s = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
        val t0 = SystemClock.elapsedRealtime()
        val ok = runCatching { sc.startScan(null, s, crowdCallback) }.isSuccess
        delay(BLE_CROWD_SCAN_MS)
        runCatching { sc.stopScan(crowdCallback) }
        if (ok) synchronized(this) { acc(stamp()).bleScanMs += SystemClock.elapsedRealtime() - t0 }
    }

    private val crowdCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, r: ScanResult) {
            val now = System.currentTimeMillis()
            synchronized(this@SightingsEngine) {
                val oid = hmac8(dayKey(now), macBytes(r.device.address))
                acc(stamp()).ble.add(oid)
            }
        }
    }

    /** Hook from HoneAnnounceScanner: stamp each Flipper frame heard with this phone's location. */
    fun onFlipperFrame(devId: String, kind: String, rssi: Int) {
        if (appCtx == null) return
        val s = stamp()
        val now = System.currentTimeMillis()
        val w: Long
        val write: Boolean
        synchronized(this) {
            acc(s).flippers.add(devId)
            w = window
            val k = "$devId/$kind"
            write = kind != "announce" || now - (flipperLast[k] ?: 0) > 60_000
            if (write) flipperLast[k] = now
        }
        if (write) runCatching { store?.putFlipperFrame(devId, kind, w, s.cell9, s.cellf, s.acc, rssi, now) }
    }

    // ── Wi-Fi ────────────────────────────────────────────────────────────────────────
    private fun throttled(c: Context): Boolean? = if (Build.VERSION.SDK_INT >= 30)
        runCatching { c.getSystemService(WifiManager::class.java)?.isScanThrottleEnabled }.getOrNull() else true

    private val wifiReceiver = object : BroadcastReceiver() {
        override fun onReceive(ctx: Context, intent: Intent) { scope.launch { ingestWifi(ctx) } }
    }

    private fun registerWifi(c: Context) {
        val f = IntentFilter(WifiManager.SCAN_RESULTS_AVAILABLE_ACTION)
        if (Build.VERSION.SDK_INT >= 33) c.registerReceiver(wifiReceiver, f, Context.RECEIVER_NOT_EXPORTED)
        else c.registerReceiver(wifiReceiver, f)
    }

    @Suppress("DEPRECATION")
    private suspend fun wifiLoop(c: Context) {
        val wm = c.getSystemService(WifiManager::class.java) ?: return
        var lastReq = 0L
        while (true) {
            val period = when {
                throttled(c) == false -> WIFI_UNTHROTTLED_MS
                uiVisible -> WIFI_FOREGROUND_MS
                else -> WIFI_BACKGROUND_MS
            }
            val now = SystemClock.elapsedRealtime()
            if (hasLocation(c) && (lastReq == 0L || now - lastReq >= period)) {
                lastReq = now
                // startScan() is deprecated but is the only way to ask; a refusal (throttled)
                // just means we use whatever results the system's own scans produce.
                if (runCatching { wm.startScan() }.getOrDefault(false)) wifiOk++ else wifiRefused++
            }
            ingestWifi(c)                       // passive: pick up anyone's recent scan
            delay(15_000)
        }
    }

    private var lastIngestTs = 0L

    @SuppressLint("MissingPermission")
    @Synchronized
    private fun ingestWifi(c: Context) {
        if (!hasLocation(c)) return
        val wm = c.getSystemService(WifiManager::class.java) ?: return
        val results = runCatching { wm.scanResults }.getOrNull() ?: return
        val nowUs = SystemClock.elapsedRealtimeNanos() / 1000
        // Only results from scans finished in the last 2 min, and only once per scan.
        val fresh = results.filter { nowUs - it.timestamp < 120_000_000L && it.timestamp > lastIngestTs }
        if (fresh.isEmpty()) return
        lastIngestTs = fresh.maxOf { it.timestamp }
        val s = stamp()
        val now = System.currentTimeMillis()
        val pk = privKey(c)
        val owned = runCatching { store?.ownRouterTags() }.getOrNull() ?: emptySet()
        val code = routerCode?.takeIf { now < it.second }?.first
        var mobileNew = 0
        run {
            val a = acc(s)
            a.wifiScans++
            val kd = dayKey(now)
            for (r in fresh) {
                @Suppress("DEPRECATION") val bssid = r.BSSID ?: continue
                val mac = macBytes(bssid)
                val oid = hmac8(kd, mac)
                val band = bandOf(r.frequency) ?: continue
                val prev = a.aps[oid]
                if (prev == null) a.aps[oid] = Ap(r.level, channelOf(r.frequency), band)
                else if (r.level > prev.rssi) prev.rssi = r.level
                // Mobile AP: same oid in a second cell today. Local event only.
                s.cell9?.let { cell ->
                    val cells = mobileCells.getOrPut(oid) { HashSet() }
                    cells.add(cell)
                    if (cells.size >= 2 && a.mobile.add(oid)) mobileNew++
                }
                // Private layer: the owner's registered routers, keyed by HMAC(K_priv, bssid).
                val tag = hmac8(pk, mac)
                @Suppress("DEPRECATION") val ssid = r.SSID ?: ""
                if (code != null && ssid.contains(code)) {
                    runCatching { store?.registerRouter(tag, "router ${tag.take(4)}") }
                    routerCode = null
                    Log.i(TAG, "own router registered by code proof")
                }
                if ((tag in owned) && s.cellf != null) runCatching { store?.putRouterCoverage(tag, window, s.cellf, r.level) }
            }
        }
        if (mobileNew > 0) Log.i(TAG, "mobile AP present in current cell (+$mobileNew)")
        Log.i(TAG, "wifi scan: ${fresh.size} APs, located=${s.cell9 != null}")
    }

    /** Start a router registration: put this code in the router's SSID for a few minutes. */
    fun beginRouterRegistration(): String {
        val code = "HONE" + (1000..9999).random()
        routerCode = code to System.currentTimeMillis() + 10 * 60_000L
        return code
    }

    fun bandOf(mhz: Int): String? = when (mhz) {
        in 2400..2500 -> "2g"
        in 4900..5899 -> "5g"
        in 5925..7125 -> "6g"
        else -> null
    }

    fun channelOf(mhz: Int): Int = when (mhz) {
        2484 -> 14
        in 2400..2500 -> (mhz - 2407) / 5
        in 4900..5899 -> (mhz - 5000) / 5
        in 5925..7125 -> (mhz - 5950) / 5
        else -> 0
    }

    fun storeOrNull(): SightingStore? = store
}

/** Fires at each UTC quarter-hour so the rid and the advertising address rotate on time. */
class WindowAlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) { SightingsEngine.tick() }
}

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
import android.os.ParcelUuid
import android.os.SystemClock
import android.util.Log
import com.google.android.gms.common.ConnectionResult
import com.google.android.gms.common.GoogleApiAvailability
import com.google.android.gms.location.LocationCallback
import com.google.android.gms.location.LocationRequest
import com.google.android.gms.location.LocationResult
import com.google.android.gms.location.LocationServices
import com.google.android.gms.location.Priority
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
import kotlin.math.abs

/** What the Sensors screen shows for the current 15-min window. */
data class LiveCoverage(
    val running: Boolean = false,
    val window: Long = 0,
    val windowEndsMs: Long = 0,
    val cell: String? = null,               // cell at the resolution the current fix supports
    val cellRes: Int? = null,
    val locAccM: Float? = null,
    val locAgeS: Long? = null,
    val locSrc: String = "",
    val apCount: Int = 0,
    val apByBand: Map<String, Int> = emptyMap(),
    val bandsSupported: List<String> = emptyList(),
    val bleCrowd: Int = 0,
    val honeDevices: Int = 0,
    val mobileAps: Int = 0,
    val wifiScansOk: Int = 0,
    val wifiScansRefused: Int = 0,
    val wifiThrottled: Boolean? = null,
    val lidPrefix: String = "",
    val advertising: Boolean = false,
    val advertMode: String = "",
    val seedDerived: Boolean = false,
    val issues: List<String> = emptyList(),
    val summariesStored: Int = 0,
    val summariesQueued: Int = 0,
    val fingerprintCells: Int = 0,
    val routerPending: String? = null,
)

/**
 * Phone as a standalone sensor (spec §1a, §1b, §2a, phone side of §2c). See docs/SIGHTINGS.md.
 *
 * Per UTC 15-min window it accumulates, per H3 cell at the resolution each reading's
 * accuracy supports: Wi-Fi APs (oid, rssi, channel, band), BLE crowd, HONE devices heard,
 * mobile-AP events, and the location samples with their accuracy (for inverse-variance
 * weighting downstream). K_day (oids) is RAM only. Sightings of other HONE devices are
 * sealed to that device's per-window key and stored under its lookup id; this phone keeps
 * no plaintext location of anyone else's device.
 */
object SightingsEngine {
    private const val TAG = "HoneSight"
    const val INFO_SIGHTING = "hone-sighting-v1"
    private const val BLE_CROWD_EVERY_MS = 60_000L
    private const val BLE_CROWD_SCAN_MS = 8_000L
    private const val WIFI_UNTHROTTLED_MS = 120_000L
    private const val WIFI_FOREGROUND_MS = 30_000L
    private const val WIFI_BACKGROUND_MS = 30 * 60_000L
    private const val LOC_MAX_AGE_MS = 5 * 60_000L
    private const val FP_TRUTH_M = 20f        // fingerprint ground truth: fixes this good or better
    private const val FP_FIX_ACC_M = 35f      // accuracy assigned to a fingerprint match

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var jobs = listOf<Job>()
    @Volatile private var appCtx: Context? = null
    private var store: SightingStore? = null

    private val _live = MutableStateFlow(LiveCoverage())
    val live: StateFlow<LiveCoverage> = _live.asStateFlow()

    @Volatile var uiVisible = false

    // ── RAM-only day key ──────────────────────────────────────────────────────────
    private var kDay: ByteArray? = null
    private var kDayIndex = Long.MIN_VALUE
    private val mobileCells = HashMap<String, MutableSet<String>>()

    private fun dayKey(nowMs: Long): ByteArray {
        val day = Math.floorDiv(nowMs, 86_400_000L)
        if (day != kDayIndex || kDay == null) {
            kDay = ByteArray(32).also { SecureRandom().nextBytes(it) }
            kDayIndex = day
            mobileCells.clear()
        }
        return kDay!!
    }

    private fun hmac8(key: ByteArray, data: ByteArray): String = IdKeys.hmac(key, data).copyOf(8).hex()

    private fun macBytes(mac: String): ByteArray =
        runCatching { mac.split(":").map { it.toInt(16).toByte() }.toByteArray() }
            .getOrDefault(mac.lowercase().toByteArray())

    // Persistent private-layer key (owner's routers, fingerprints). Never leaves the device.
    private var kPriv: ByteArray? = null
    private fun privKey(ctx: Context): ByteArray {
        kPriv?.let { return it }
        val f = File(ctx.filesDir, "hone-node/priv_layer_key")
        val k = if (f.exists() && f.length() == 32L) f.readBytes()
            else ByteArray(32).also { SecureRandom().nextBytes(it); f.parentFile?.mkdirs(); f.writeBytes(it) }
        kPriv = k; return k
    }

    // ── Identity ─────────────────────────────────────────────────────────────────
    @Volatile private var seedDerived = false
    @Volatile private var wkey: IdKeys.WindowKey? = null

    /** Re-read the seed (after a hide key import): one-time key change. */
    fun reloadIdentity() {
        val c = appCtx ?: return
        val (k, src) = RoleKeys.thisWindowKey(c, window)
        seedDerived = src != "provisional"
        wkey = k
        IdAdvertiser.stop(c)
        IdAdvertiser.ensure(c, wkey!!)
        Log.i(TAG, "identity: $src")
    }

    // ── Location ─────────────────────────────────────────────────────────────────
    @Volatile private var loc: Location? = null
    @Volatile private var locSrc = ""
    fun lastFix(): Location? = loc
    @Volatile private var fpFix: Location? = null        // from Wi-Fi fingerprints
    private val locListener = LocationListener { l -> onLocation(l, if (l.provider == "gps") "gps" else l.provider ?: "lm") }
    private var gmsCallback: LocationCallback? = null

    private fun onLocation(l: Location, src: String) {
        val cur = loc
        if (cur == null || l.time > cur.time + 30_000 || l.accuracy <= cur.accuracy) {
            loc = l; locSrc = src; stampCache = null
        }
    }

    private fun ageMs(l: Location) = (SystemClock.elapsedRealtimeNanos() - l.elapsedRealtimeNanos) / 1_000_000

    /** Best current estimate: the fresher/better of the radio fix and a fingerprint match. */
    private fun bestFix(): Pair<Location, String>? {
        val a = loc?.takeIf { ageMs(it) in 0..LOC_MAX_AGE_MS }?.let { it to locSrc }
        val b = fpFix?.takeIf { ageMs(it) in 0..120_000 }?.let { it to "wifi-fp" }
        return listOfNotNull(a, b).minByOrNull { it.first.accuracy }
    }

    /** Cells for one reading: `cell` at the accuracy-matched resolution (public), `cellf` res 11 (private). */
    private data class Stamp(val cell: String?, val cellf: String?, val acc: Float?, val src: String)

    @Volatile private var stampCache: Pair<Long, Stamp>? = null

    private fun stamp(): Stamp {
        val now = SystemClock.elapsedRealtime()
        stampCache?.let { (t, st) -> if (now - t < 5_000) return st }
        val f = bestFix()
        val st = if (f == null) Stamp(null, null, null, "") else {
            val (l, src) = f
            val r = Cells.resFor(l.accuracy)
            Stamp(r?.let { Cells.cell(l.latitude, l.longitude, it) },
                Cells.cell(l.latitude, l.longitude, Cells.PRIVATE_RES), l.accuracy, src)
        }
        stampCache = now to st
        return st
    }

    fun hasLocation(ctx: Context) =
        ctx.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

    fun hasBackgroundLocation(ctx: Context) = Build.VERSION.SDK_INT < 29 ||
        ctx.checkSelfPermission(Manifest.permission.ACCESS_BACKGROUND_LOCATION) == PackageManager.PERMISSION_GRANTED

    private fun hasGms(ctx: Context) = runCatching {
        GoogleApiAvailability.getInstance().isGooglePlayServicesAvailable(ctx) == ConnectionResult.SUCCESS
    }.getOrDefault(false)

    @SuppressLint("MissingPermission")
    private fun startLocation(ctx: Context) {
        if (!hasLocation(ctx)) return
        val lm = ctx.getSystemService(LocationManager::class.java)
        if (hasGms(ctx)) runCatching {
            // Play services fused location, high accuracy (GPS + Wi-Fi + cell + sensors), 60 s.
            val cb = object : LocationCallback() {
                override fun onLocationResult(r: LocationResult) { r.lastLocation?.let { onLocation(it, "fused-gms") } }
            }
            val req = LocationRequest.Builder(Priority.PRIORITY_HIGH_ACCURACY, 60_000L)
                .setMinUpdateIntervalMillis(20_000L).setMinUpdateDistanceMeters(0f).build()
            LocationServices.getFusedLocationProviderClient(ctx).requestLocationUpdates(req, cb, Looper.getMainLooper())
            gmsCallback = cb
            Log.i(TAG, "location via Play services fused, high accuracy")
        }.onFailure { Log.w(TAG, "GMS location failed: ${it.message}") }
        if (gmsCallback == null && lm != null) runCatching {
            val providers = lm.allProviders
            val main = if (Build.VERSION.SDK_INT >= 31 && LocationManager.FUSED_PROVIDER in providers)
                LocationManager.FUSED_PROVIDER else if (LocationManager.NETWORK_PROVIDER in providers)
                LocationManager.NETWORK_PROVIDER else LocationManager.GPS_PROVIDER
            lm.getLastKnownLocation(main)?.let { onLocation(it, main) }
            lm.requestLocationUpdates(main, 60_000L, 10f, locListener, Looper.getMainLooper())
            Log.i(TAG, "location via LocationManager $main")
        }.onFailure { Log.w(TAG, "location updates failed: ${it.message}") }
        // Passive: reuse any GPS fix another component pays for.
        runCatching { lm?.requestLocationUpdates(LocationManager.PASSIVE_PROVIDER, 10_000L, 0f, locListener, Looper.getMainLooper()) }
    }

    // ── Per-window accumulation ──────────────────────────────────────────────────
    private class Ap(var rssi: Int, val channel: Int, val band: String)
    private class Acc {
        val aps = HashMap<String, Ap>()
        val ble = HashSet<String>()
        val hone = HashSet<String>()
        val mobile = HashSet<String>()
        val accs = ArrayList<Float>()             // accuracy (m) of each reading in this cell
        val src = HashMap<String, Int>()
        var wifiScans = 0
        var bleScanMs = 0L
    }
    private var window = Long.MIN_VALUE
    private val accs = HashMap<String, Acc>()            // cell ("" = no fix) -> acc
    private val sightBest = HashMap<String, Pair<String?, Int>>()  // lid -> (cellf, rssi) last sealed
    private val flipperLast = HashMap<String, Long>()
    private var wifiOk = 0; private var wifiRefused = 0
    @Volatile private var routerCode: Pair<String, Long>? = null

    /** The accumulator for one reading; records its accuracy as one location sample. */
    private fun acc(s: Stamp): Acc = accs.getOrPut(s.cell ?: "") { Acc() }.also { a ->
        s.acc?.let { a.accs.add(it); a.src.merge(s.src, 1, Int::plus) }
    }

    // ── Lifecycle ────────────────────────────────────────────────────────────────
    @Synchronized
    fun start(ctx: Context) {
        if (appCtx != null) return
        val c = ctx.applicationContext
        appCtx = c
        store = SightingStore(c)

        rollWindow(c, System.currentTimeMillis())
        startLocation(c)
        startIdScan(c)
        registerWifi(c)
        jobs = listOf(
            scope.launch { while (isActive) { tick(); delay(5_000) } },
            scope.launch { while (isActive) { crowdScan(c); delay(BLE_CROWD_EVERY_MS - BLE_CROWD_SCAN_MS) } },
            scope.launch { wifiLoop(c) },
            scope.launch { while (isActive) { runCatching { FindMy.refreshShared(c, store!!) }; delay(30 * 60_000L) } },
        )
        Log.i(TAG, "sightings engine started; h3=${Cells.available()} gms=${gmsCallback != null} seed=${if (seedDerived) "derived" else "provisional"}")
    }

    @SuppressLint("MissingPermission")
    @Synchronized
    fun stop() {
        val c = appCtx ?: return
        jobs.forEach { it.cancel() }; jobs = emptyList()
        finalizeWindow()
        IdAdvertiser.stop(c)
        runCatching { c.getSystemService(LocationManager::class.java)?.removeUpdates(locListener) }
        gmsCallback?.let { cb -> runCatching { LocationServices.getFusedLocationProviderClient(c).removeLocationUpdates(cb) } }
        gmsCallback = null
        runCatching { c.unregisterReceiver(wifiReceiver) }
        runCatching { bleScanner(c)?.stopScan(idCallback) }
        runCatching { bleScanner(c)?.stopScan(crowdCallback) }
        cancelAlarm(c)
        appCtx = null
        _live.value = _live.value.copy(running = false, advertising = false)
    }

    fun tick() {
        val c = appCtx ?: return
        rollWindow(c, System.currentTimeMillis())
        wkey?.let { IdAdvertiser.ensure(c, it) }
        publishLive(c)
    }

    @Synchronized
    private fun rollWindow(c: Context, nowMs: Long) {
        val w = IdKeys.window(nowMs)
        if (w == window) return
        if (window != Long.MIN_VALUE) finalizeWindow()
        window = w
        accs.clear(); sightBest.clear()
        dayKey(nowMs)
        wkey = RoleKeys.thisWindowKey(c, w).let { (k, src) -> seedDerived = src != "provisional"; k }
        IdAdvertiser.ensure(c, wkey!!)              // new key + new advertising set(s) (address)
        scheduleAlarm(c, IdKeys.windowStartMs(w + 1))
        store?.prune(w)
        Log.i(TAG, "window $w")
    }

    @Synchronized
    private fun finalizeWindow() {
        val st = store ?: return
        if (window == Long.MIN_VALUE) return
        for ((cell, a) in accs) {
            if (a.aps.isEmpty() && a.ble.isEmpty() && a.hone.isEmpty()) continue
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

    /** Public summary for one (cell, window), schema v2 in docs/SIGHTINGS.md. No identities. */
    private fun summaryJson(cell: String, a: Acc): JSONObject {
        val byBand = a.aps.values.groupingBy { it.band }.eachCount()
        val hist = IntArray(7)
        a.aps.values.forEach { hist[rssiBucket(it.rssi)]++ }
        val rssis = a.aps.values.map { it.rssi }.sorted()
        val chans = JSONObject()
        a.aps.values.groupingBy { "${it.band}:${it.channel}" }.eachCount().toSortedMap().forEach { (k, v) -> chans.put(k, v) }
        val acc = a.accs.sorted()
        val weight = acc.sumOf { 1.0 / (it.toDouble() * it.toDouble()) }
        return JSONObject()
            .put("v", 2).put("cell", cell).put("res", if (cell.isEmpty()) JSONObject.NULL else Cells.res(cell)).put("window", window)
            .put("loc", JSONObject()
                .put("samples", acc.size)
                .put("acc_m_min", acc.firstOrNull()?.let { Math.round(it) } ?: JSONObject.NULL)
                .put("acc_m_p50", if (acc.isEmpty()) JSONObject.NULL else Math.round(acc[acc.size / 2]))
                .put("weight", if (acc.isEmpty()) 0.0 else "%.6g".format(weight).toDouble())
                .put("src", JSONObject(a.src as Map<*, *>)))
            .put("wifi", JSONObject()
                .put("ap_count", a.aps.size)
                .put("by_band", JSONObject().apply { listOf("2g", "5g", "6g").forEach { put(it, byBand[it] ?: 0) } })
                .put("bands_supported", JSONArray(appCtx?.let { bandsSupported(it) } ?: emptyList<String>()))
                .put("rssi_hist", JSONArray(hist.toList()))
                .put("rssi_p50", if (rssis.isEmpty()) JSONObject.NULL else rssis[rssis.size / 2])
                .put("rssi_max", rssis.lastOrNull() ?: JSONObject.NULL)
                .put("channels", chans)
                .put("scans", a.wifiScans))
            .put("ble", JSONObject().put("crowd", a.ble.size).put("scan_s", a.bleScanMs / 1000).put("filter", "v1"))
            .put("hone", JSONObject().put("devices", a.hone.size))
            .put("mobile_ap", a.mobile.size)
    }

    private fun rssiBucket(r: Int): Int = when {
        r < -90 -> 0; r <= -81 -> 1; r <= -71 -> 2; r <= -61 -> 3; r <= -51 -> 4; r <= -41 -> 5; else -> 6
    }

    private fun publishLive(c: Context) {
        val s = stamp()
        val f = bestFix()
        val issues = buildList {
            if (!hasLocation(c)) add("location denied: readings kept unlocated, nothing queued")
            else if (!hasBackgroundLocation(c)) add("location only while app open (allow 'all the time')")
            if (!Cells.available()) add("H3 unavailable")
            if (!seedDerived) add("no hardware grant: provisional identity (Find my things > hardware grant)")
            if (!IdAdvertiser.hasPermission(c)) add("Bluetooth advertise permission missing")
            else if (IdAdvertiser.lastError.isNotEmpty()) add(IdAdvertiser.lastError)
        }
        val (n, q) = runCatching { store?.summaryCounts() }.getOrNull() ?: (0 to 0)
        val fpc = runCatching { store?.fingerprintCells() }.getOrNull() ?: 0
        val snap = synchronized(this) {
            val a = accs[s.cell ?: ""]
            LiveCoverage(
                running = true, window = window, windowEndsMs = IdKeys.windowStartMs(window + 1),
                cell = s.cell, cellRes = s.cell?.let { Cells.res(it) }, locAccM = f?.first?.accuracy,
                locAgeS = f?.first?.let { ageMs(it) / 1000 }, locSrc = f?.second ?: "",
                apCount = a?.aps?.size ?: 0,
                apByBand = a?.aps?.values?.groupingBy { it.band }?.eachCount() ?: emptyMap(),
                bandsSupported = bandsSupported(c),
                bleCrowd = a?.ble?.size ?: 0,
                honeDevices = accs.values.flatMap { it.hone }.toSet().size,
                mobileAps = a?.mobile?.size ?: 0,
                wifiScansOk = wifiOk, wifiScansRefused = wifiRefused,
                wifiThrottled = throttled(c),
                lidPrefix = wkey?.lid?.copyOf(4)?.hex() ?: "",
                advertising = IdAdvertiser.advertising, advertMode = IdAdvertiser.mode,
                seedDerived = seedDerived,
                issues = issues, summariesStored = n, summariesQueued = q, fingerprintCells = fpc,
                routerPending = routerCode?.takeIf { System.currentTimeMillis() < it.second }?.first,
            )
        }
        _live.value = snap
    }

    // ── Boundary alarm ───────────────────────────────────────────────────────────
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

    // ── BLE ──────────────────────────────────────────────────────────────────────
    private fun bleAdapter(c: Context) = c.getSystemService(BluetoothManager::class.java)?.adapter
    private fun bleScanner(c: Context) = bleAdapter(c)?.bluetoothLeScanner

    private fun hasScan(c: Context) = if (Build.VERSION.SDK_INT >= 31)
        c.checkSelfPermission(Manifest.permission.BLUETOOTH_SCAN) == PackageManager.PERMISSION_GRANTED
    else hasLocation(c)

    /** HONE id adverts ('hK'): hardware filter, low power, always on, legacy + extended. */
    @SuppressLint("MissingPermission")
    private fun startIdScan(c: Context) {
        if (!hasScan(c)) return
        val f = ScanFilter.Builder().setManufacturerData(IdAdvertiser.COMPANY,
            IdAdvertiser.PREFIX, byteArrayOf(0xFF.toByte(), 0xFF.toByte())).build()
        val sb = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_POWER)
        if (bleAdapter(c)?.isLeExtendedAdvertisingSupported == true)
            sb.setLegacy(false).setPhy(ScanSettings.PHY_LE_ALL_SUPPORTED)
        runCatching { bleScanner(c)?.startScan(listOf(f), sb.build(), idCallback) }
            .onFailure { Log.w(TAG, "id scan: ${it.message}") }
    }

    private val frags = HashMap<String, Pair<Long, Array<ByteArray?>>>()   // "win8/tag" -> (ms, parts)

    private val idCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, r: ScanResult) {
            val md = r.scanRecord?.getManufacturerSpecificData(IdAdvertiser.COMPANY) ?: return
            if (md.size < 7 || md[0] != IdAdvertiser.PREFIX[0] || md[1] != IdAdvertiser.PREFIX[1] || md[2] != IdAdvertiser.VERSION) return
            val idx = (md[4].toInt() and 0xF0) shr 4
            val total = md[4].toInt() and 0x0F
            val body = md.copyOfRange(7, md.size)
            val pk: ByteArray = when {
                total == 1 && idx == 0 && body.size == 32 -> body
                total == 2 && idx < 2 && body.size == 16 -> {
                    val key = "${md[3]}/${md[5]}/${md[6]}"
                    synchronized(frags) {
                        val now = SystemClock.elapsedRealtime()
                        frags.entries.removeIf { now - it.value.first > 60_000 }
                        val parts = frags.getOrPut(key) { now to arrayOfNulls(2) }.second
                        parts[idx] = body
                        if (parts[0] == null || parts[1] == null) return
                        parts[0]!! + parts[1]!!
                    }
                }
                else -> return
            }
            onPeerKey(pk, md[3].toInt() and 0xFF, r.rssi)
        }
        override fun onScanFailed(errorCode: Int) { Log.w(TAG, "id scan failed $errorCode") }
    }

    /** Another HONE device's pk_w heard: seal (cell, window, rssi, accuracy) to it. */
    private fun onPeerKey(pk: ByteArray, win8: Int, rssi: Int) {
        if (wkey?.pk?.contentEquals(pk) == true) return
        val lid = IdKeys.lid(pk).hex()
        val s = stamp()
        val now = System.currentTimeMillis()
        val w: Long
        val write: Boolean
        synchronized(this) {
            acc(s).hone.add(lid)
            w = window
            val prev = sightBest[lid]
            write = prev == null || prev.first != s.cellf || rssi >= prev.second + 3
            if (write) sightBest[lid] = s.cellf to maxOf(rssi, prev?.second ?: rssi)
        }
        if (!write) return
        val rec = JSONObject().put("w", w).put("w8", win8).put("t", now).put("r", rssi)
            .put("c", s.cellf ?: JSONObject.NULL).put("cp", s.cell ?: JSONObject.NULL)
            .put("a", s.acc?.let { Math.round(it) } ?: JSONObject.NULL).put("s", s.src)
        val blob = runCatching { IdKeys.seal(pk, INFO_SIGHTING, rec.toString().toByteArray(), aad = lid.unhex()) }.getOrNull() ?: return
        runCatching { store?.putEncSighting(lid, w, blob, now) }
        Log.i(TAG, "sighting sealed lid=${lid.take(8)}.. rssi=$rssi")
    }

    /**
     * BLE crowd. Android drops unfiltered scan results with the screen off, so the scan uses
     * broad hardware filters that still match most phones, wearables and trackers (v1 list
     * below). The same filter is used with the screen on, so counts are comparable.
     */
    private val crowdFilters: List<ScanFilter> by lazy {
        val mfr = listOf(0x004C /*Apple*/, 0x0006 /*Microsoft*/, 0x0075 /*Samsung*/, 0x00E0 /*Google*/,
            0x0087 /*Garmin*/, 0x0157 /*Huami/Amazfit*/, 0x038F /*Xiaomi*/, 0x0171 /*Amazon*/)
        val svc = listOf("FE2C" /*Fast Pair*/, "FEAA" /*Eddystone, Find Hub network*/, "FD6F" /*Exposure Notification*/,
            "FD5A" /*Samsung SmartTag*/, "FEED", "FEEC" /*Tile*/, "180D" /*heart rate*/, "FE07" /*Sonos*/,
            "FE9F" /*Google*/, "FD44" /*Apple Find My accessories*/)
        mfr.map { ScanFilter.Builder().setManufacturerData(it, ByteArray(0)).build() } +
            svc.map { ScanFilter.Builder().setServiceUuid(ParcelUuid.fromString("0000$it-0000-1000-8000-00805F9B34FB")).build() } +
            svc.map { ScanFilter.Builder().setServiceData(ParcelUuid.fromString("0000$it-0000-1000-8000-00805F9B34FB"), ByteArray(0)).build() }
    }

    @SuppressLint("MissingPermission")
    private suspend fun crowdScan(c: Context) {
        if (!hasScan(c)) { delay(BLE_CROWD_SCAN_MS); return }
        val sc = bleScanner(c) ?: run { delay(BLE_CROWD_SCAN_MS); return }
        val sb = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY)
        if (bleAdapter(c)?.isLeExtendedAdvertisingSupported == true) sb.setLegacy(false)
        val t0 = SystemClock.elapsedRealtime()
        val ok = runCatching { sc.startScan(crowdFilters, sb.build(), crowdCallback) }.isSuccess
        delay(BLE_CROWD_SCAN_MS)
        runCatching { sc.stopScan(crowdCallback) }
        if (ok) synchronized(this) { acc(stamp()).bleScanMs += SystemClock.elapsedRealtime() - t0 }
    }

    @Volatile var crowdResults = 0L; private set

    private val crowdCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, r: ScanResult) {
            crowdResults++
            val now = System.currentTimeMillis()
            val s = stamp()
            synchronized(this@SightingsEngine) {
                val a = accs.getOrPut(s.cell ?: "") { Acc() }   // not a location sample per advert
                a.ble.add(hmac8(dayKey(now), macBytes(r.device.address)))
            }
        }
    }

    fun onFlipperFrame(devId: String, kind: String, rssi: Int) {
        if (appCtx == null) return
        val s = stamp()
        val now = System.currentTimeMillis()
        val w: Long
        val write: Boolean
        synchronized(this) {
            acc(s).hone.add("flipper:$devId")
            w = window
            val k = "$devId/$kind"
            write = kind != "announce" || now - (flipperLast[k] ?: 0) > 60_000
            if (write) flipperLast[k] = now
        }
        if (write) runCatching { store?.putFlipperFrame(devId, kind, w, s.cell, s.cellf, s.acc, rssi, now) }
    }

    // ── Wi-Fi ────────────────────────────────────────────────────────────────────
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
                if (runCatching { wm.startScan() }.getOrDefault(false)) wifiOk++ else wifiRefused++
            }
            ingestWifi(c)
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
        val fresh = results.filter { nowUs - it.timestamp < 120_000_000L && it.timestamp > lastIngestTs }
        if (fresh.isEmpty()) return
        lastIngestTs = fresh.maxOf { it.timestamp }
        val now = System.currentTimeMillis()
        val pk = privKey(c)
        @Suppress("DEPRECATION")
        val tagged = fresh.mapNotNull { r -> r.BSSID?.let { b -> Triple(r, macBytes(b), hmac8(pk, macBytes(b))) } }
        fingerprint(tagged.associate { it.third to it.first.level })
        val s = stamp()
        val owned = runCatching { store?.ownRouterTags() }.getOrNull() ?: emptySet()
        val code = routerCode?.takeIf { now < it.second }?.first
        var mobileNew = 0
        val a = acc(s)
        a.wifiScans++
        val kd = dayKey(now)
        for ((r, mac, tag) in tagged) {
            val oid = hmac8(kd, mac)
            val band = bandOf(r.frequency) ?: continue
            val prev = a.aps[oid]
            if (prev == null) a.aps[oid] = Ap(r.level, channelOf(r.frequency), band)
            else if (r.level > prev.rssi) prev.rssi = r.level
            s.cell?.takeIf { s.acc != null && s.acc <= Cells.EDGE_M[Cells.PUBLIC_RES] }?.let { cell ->
                // Mobile AP: same oid in two res-9-or-finer cells today (well-located readings only).
                val cells = mobileCells.getOrPut(oid) { HashSet() }
                cells.add(cell)
                if (cells.size >= 2 && a.mobile.add(oid)) mobileNew++
            }
            @Suppress("DEPRECATION") val ssid = r.SSID ?: ""
            if (code != null && ssid.contains(code)) {
                runCatching { store?.registerRouter(tag, "router ${tag.take(4)}") }
                routerCode = null
                Log.i(TAG, "own router registered by code proof")
            }
            if ((tag in owned) && s.cellf != null) runCatching { store?.putRouterCoverage(tag, window, s.cellf, r.level) }
        }
        if (mobileNew > 0) Log.i(TAG, "mobile AP present in current cell (+$mobileNew)")
        Log.i(TAG, "wifi scan: ${fresh.size} APs, cell=${s.cell?.let { "r" + Cells.res(it) } ?: "none"} acc=${s.acc?.toInt()} src=${s.src}")
    }

    /**
     * Private Wi-Fi fingerprint positioning. A scan taken with a fix of <= 20 m (GPS-grade)
     * trains the res-11 cell's (AP tag -> mean RSSI) fingerprint. A scan without such a fix
     * is matched against stored fingerprints; a good match becomes a 35 m "wifi-fp" fix.
     */
    private fun fingerprint(obs: Map<String, Int>) {
        if (obs.size < 3) return
        val st = store ?: return
        val f = loc?.takeIf { ageMs(it) in 0..60_000 }
        if (f != null && f.accuracy <= FP_TRUTH_M) {
            Cells.cell(f.latitude, f.longitude, Cells.PRIVATE_RES)?.let { runCatching { st.addFingerprint(it, obs) } }
            return
        }
        val cand = runCatching { st.fingerprintsFor(obs.keys) }.getOrNull() ?: return
        var best: Pair<String, Double>? = null
        for ((cell, fp) in cand) {
            val common = obs.keys.intersect(fp.keys)
            if (common.size < 3) continue
            val miss = (obs.keys - fp.keys).size + (fp.keys - obs.keys).count { (fp[it] ?: -100.0) > -75 }
            val score = common.sumOf { abs(obs[it]!! - fp[it]!!) } / common.size + 4.0 * miss / (common.size + miss)
            if (best == null || score < best.second) best = cell to score
        }
        val b = best ?: return
        if (b.second > 10.0) return
        val (lat, lon) = Cells.center(b.first) ?: return
        if ((f?.accuracy ?: Float.MAX_VALUE) <= FP_FIX_ACC_M) return
        fpFix = Location("wifi-fp").apply {
            latitude = lat; longitude = lon; accuracy = FP_FIX_ACC_M; time = System.currentTimeMillis()
            elapsedRealtimeNanos = SystemClock.elapsedRealtimeNanos()
        }
        stampCache = null
        Log.i(TAG, "wifi fingerprint fix (score ${"%.1f".format(b.second)})")
    }

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
    fun currentWindow(): Long = window
}

/** Fires at each UTC quarter-hour so the key and the advertising address rotate on time. */
class WindowAlarmReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) { SightingsEngine.tick() }
}

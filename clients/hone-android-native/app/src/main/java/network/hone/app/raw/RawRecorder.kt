package network.hone.app.raw

import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.net.wifi.ScanResult
import android.os.BatteryManager
import android.os.Build
import android.util.Log
import java.io.ByteArrayOutputStream
import java.io.File
import java.security.MessageDigest
import java.time.LocalDate
import java.time.temporal.IsoFields
import java.util.zip.GZIPOutputStream

/**
 * "Send everything" (Josh 2026-11-10): the device's full raw record per 15-minute window, as
 * change-based hone.raw records ([RawFmt]). Public content: identifiers are tokens (BSSID/SSID
 * hashed with the res-7 area and ISO week, BLE MACs with a per-device daily salt), location is
 * the coarse area, human speech is never recorded (only a gap marker that speech was deleted).
 * One gzip JSONL blob per window goes to the upload queue (filesDir/hone-node/raw-queue); its
 * sha256 is the window's SENSOR_SUMMARY batch_hash ([shaFor]).
 */
object RawRecorder {
    private const val TAG = "HoneRaw"
    private val lock = Any()
    private var enc = ChangeEncoder()
    @Volatile private var app: Context? = null
    private var started = false
    private var day = ""
    private val ble = HashMap<String, Pair<Double, Map<String, Any?>>>()
    private val full = ArrayList<Map<String, Any?>>()          // full-scan log, only to measure the saving
    private var speechFromMs = 0L
    private var speechToMs = 0L
    private val sensorsFound = LinkedHashMap<String, String>()

    fun qdir(ctx: Context) = File(ctx.filesDir, "hone-node/raw-queue")
    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_raw", Context.MODE_PRIVATE)

    /** Start once per process (boot keyframe; climate sensors; device temperature). */
    fun start(ctx: Context) = synchronized(lock) {
        if (started) return
        val c = ctx.applicationContext; app = c; started = true
        val now = System.currentTimeMillis()
        day = utcDay(now)
        val lastW = prefs(c).getLong("last_window", -1)
        val w = now / 900_000
        if (lastW in 0 until w - 1) {
            // App or phone was off: an explicit gap for every family, then a keyframe.
            for (f in listOf("wifi", "ble", "env", "audio", "flipper")) enc.gap((lastW + 1) * 900_000, now, f, "app_stopped")
        }
        enc.keyframe(now, "boot")
        startClimate(c)
    }

    private fun utcDay(ms: Long) = LocalDate.ofEpochDay(Math.floorDiv(ms, 86_400_000L)).toString()
    private fun isoWeek(ms: Long): String {
        val d = LocalDate.ofEpochDay(Math.floorDiv(ms, 86_400_000L))
        return "%04d-W%02d".format(d.get(IsoFields.WEEK_BASED_YEAR), d.get(IsoFields.WEEK_OF_WEEK_BASED_YEAR))
    }
    private fun bleSalt(c: Context, ms: Long): String {
        val d = utcDay(ms); val p = prefs(c)
        if (p.getString("ble_day", "") != d) {
            val b = ByteArray(16).also { java.security.SecureRandom().nextBytes(it) }
            p.edit().putString("ble_day", d).putString("ble_salt", b.joinToString("") { "%02x".format(it) }).apply()
        }
        return p.getString("ble_salt", "")!!
    }

    // ── observations (called by the sightings engine and sound pipeline) ────────────────
    /** A finished Wi-Fi scan ([results] may be empty: no signal). */
    @Suppress("DEPRECATION")
    fun onWifiScan(results: List<ScanResult>) {
        val c = app ?: return
        val now = System.currentTimeMillis()
        val salt = AreaLocator.salt(c); val wk = isoWeek(now)
        val seen = HashMap<String, Pair<Double, Map<String, Any?>>>()
        for (r in results) {
            val b = r.BSSID ?: continue
            val a = LinkedHashMap<String, Any?>()
            a["f"] = r.frequency
            a["b"] = when (r.frequency) { in 2400..2500 -> "2g"; in 4900..5900 -> "5g"; in 5925..7125 -> "6g"; else -> "other" }
            val ssid = r.SSID ?: ""
            if (ssid.isEmpty()) a["hid"] = 1 else a["ss"] = RawFmt.token(salt, wk, "ssid", ssid, n = 8)
            if (Build.VERSION.SDK_INT >= 30) a["std"] = r.wifiStandard
            seen["wifi:" + RawFmt.token(salt, wk, b.lowercase())] = r.level.toDouble() to a
        }
        synchronized(lock) {
            if (!started) return
            if (results.isEmpty()) enc.gap(now - 60_000, now, "wifi", "no_signal")
            enc.observeSet(now, "wifi", seen)
            full.add(mapOf("t" to now, "wifi" to seen.mapValues { listOf(it.value.first, it.value.second) }))
        }
    }

    fun wifiDenied(fromMs: Long, toMs: Long) = synchronized(lock) { if (started) enc.gap(fromMs, toMs, "wifi", "permission_denied") }

    fun onBleAdvert(mac: String, rssi: Int) {
        val c = app ?: return
        val k = "ble:" + RawFmt.token(bleSalt(c, System.currentTimeMillis()), mac.lowercase())
        synchronized(lock) { val p = ble[k]; if (p == null || rssi > p.first) ble[k] = rssi.toDouble() to emptyMap() }
    }

    /** One BLE crowd scan ended: its full set is one observation. */
    fun onBleScanDone(ok: Boolean) {
        val now = System.currentTimeMillis()
        synchronized(lock) {
            if (!started) return
            if (!ok) enc.gap(now - 10_000, now, "ble", "permission_denied")
            else { if (ble.isEmpty()) enc.gap(now - 10_000, now, "ble", "no_signal"); enc.observeSet(now, "ble", HashMap(ble)) }
            full.add(mapOf("t" to now, "ble" to ble.mapValues { it.value.first }))
            ble.clear()
        }
    }

    fun onFlipperFrame(devId: String, rssi: Int) = synchronized(lock) {
        if (started) enc.observe(System.currentTimeMillis(), "flipper:$devId", rssi.toDouble())
    }

    /** A bird detection that passed the human-speech gate. */
    fun onBird(latin: String, conf: Float, model: String, tMs: Long) = synchronized(lock) {
        if (!started) return
        val k = "bird:" + RawFmt.token(latin, n = 10)
        enc.observe(tMs, k, Math.round(conf * 100) / 100.0, mapOf("species" to latin, "model" to model, "conf" to Math.round(conf * 100) / 100.0))
    }

    /** Audio that contained human speech was dropped on the phone: mark the span, keep nothing. */
    fun onSpeechDropped(tMs: Long) = synchronized(lock) {
        if (speechToMs > 0 && tMs - speechToMs < 60_000) speechToMs = tMs else { flushSpeech(); speechFromMs = tMs; speechToMs = tMs }
    }
    private fun flushSpeech() {
        if (speechToMs > 0 && started) enc.gap(speechFromMs, speechToMs, "audio", "speech_deleted")
        speechFromMs = 0; speechToMs = 0
    }

    // ── climate (real ambient sensors only) and device temperature ─────────────────────
    private val climate = mapOf(
        Sensor.TYPE_PRESSURE to "env.pressure_pa", Sensor.TYPE_RELATIVE_HUMIDITY to "env.rh_pct",
        Sensor.TYPE_AMBIENT_TEMPERATURE to "env.air_temp_c", Sensor.TYPE_LIGHT to "env.light_lux")
    private val lastSampleMs = HashMap<String, Long>()
    private val listener = object : SensorEventListener {
        override fun onSensorChanged(e: SensorEvent) {
            val k = climate[e.sensor.type] ?: return
            val now = System.currentTimeMillis()
            if (now - (lastSampleMs[k] ?: 0) < 10_000) return
            lastSampleMs[k] = now
            val v = e.values[0].toDouble().let { if (k == "env.pressure_pa") it * 100.0 else it }   // hPa -> Pa (SI)
            synchronized(lock) { if (started) enc.observe(now, k, Math.round(v * 100) / 100.0) }
        }
        override fun onAccuracyChanged(s: Sensor, a: Int) {}
    }

    private fun startClimate(c: Context) {
        val sm = c.getSystemService(SensorManager::class.java) ?: return
        for ((type, k) in climate) {
            val s = sm.getDefaultSensor(type)
            if (s == null) { sensorsFound[k] = "absent"; continue }
            sensorsFound[k] = "${s.vendor} ${s.name} (res ${s.resolution})"
            runCatching { sm.registerListener(listener, s, SensorManager.SENSOR_DELAY_NORMAL, 10_000_000) }
        }
        sensorsFound["dev.temp_c"] = "battery thermistor (device temperature, not air)"
    }

    private fun deviceTemp(c: Context) {
        val i: Intent = c.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED)) ?: return
        val t = i.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, Int.MIN_VALUE)
        if (t != Int.MIN_VALUE) enc.observe(System.currentTimeMillis(), "dev.temp_c", t / 10.0)
    }

    // ── window close ───────────────────────────────────────────────────────────────────
    /**
     * Window [w] ended: markers, gaps, header; queue the blob. Returns its sha256 (also kept for
     * [shaFor]). [area] is the window's on-device coarse area.
     */
    fun closeWindow(w: Long, area: AreaLocator.Area): String? {
        val c = app ?: return null
        val tEnd = (w + 1) * 900_000 - 1
        val recs: List<Map<String, Any?>>
        val fullLog: List<Map<String, Any?>>
        synchronized(lock) {
            if (!started) return null
            runCatching { deviceTemp(c) }
            flushSpeech()
            for ((k, v) in sensorsFound) if (v == "absent") enc.gap(w * 900_000, tEnd, k, "sensor_absent")
            enc.endWindow(tEnd, listOf("wifi", "ble", "flipper", "bird"))
            val hdr = linkedMapOf<String, Any?>(
                "s" to RawFmt.SCHEMA_HDR, "t" to RawFmt.rfc3339(w * 900_000), "win" to w,
                "dev" to network.hone.app.telemetry.Telemetry.deviceId(c),
                "owner" to (network.hone.app.telemetry.Telemetry.seatAccount(c) ?: ""),
                "cls" to "phone", "fw" to network.hone.app.BuildConfig.VERSION_NAME, "model" to "${Build.MANUFACTURER} ${Build.MODEL}",
                "sensors" to LinkedHashMap(sensorsFound), "area" to area.toMap(), "deadbands" to RawFmt.DEADBANDS.mapValues {
                    if (it.value.rel > 0) mapOf("rel" to it.value.rel) else mapOf("abs" to it.value.abs) })
            recs = listOf(hdr) + enc.take()
            fullLog = ArrayList(full); full.clear()
            val d = utcDay(System.currentTimeMillis())
            if (d != day) { day = d; enc.keyframe(System.currentTimeMillis(), "day") }
            prefs(c).edit().putLong("last_window", w).apply()
        }
        val body = gzip(RawFmt.jsonl(recs))
        val sha = MessageDigest.getInstance("SHA-256").digest(body).joinToString("") { "%02x".format(it) }
        val q = qdir(c); q.mkdirs()
        val base = File(q, "$w-${sha.take(16)}")
        File(base.path + ".gz.tmp").apply { writeBytes(body); renameTo(File(base.path + ".gz")) }
        File(base.path + ".json").writeText(RawFmt.json(mapOf("window" to w, "cell" to area.cell, "sha256" to sha)))
        prefs(c).edit().putString("sha_$w", sha).apply()
        // Size of the change-based blob against a full-scan log of the same window.
        runCatching {
            val fullRaw = RawFmt.jsonl(fullLog)
            val line = RawFmt.json(mapOf("win" to w, "records" to recs.size, "delta_gz" to body.size, "delta_raw" to RawFmt.jsonl(recs).size,
                "full_gz" to gzip(fullRaw).size, "full_raw" to fullRaw.size, "obs" to fullLog.size))
            File(c.filesDir, "hone-node/raw_sizes.jsonl").appendText(line + "\n")
            prefs(c).edit().putString("last_sizes", line).apply()
        }
        Log.i(TAG, "window $w: ${recs.size} records, ${body.size} B, area ${area.src}")
        return sha
    }

    /** sha256 of window [w]'s raw blob, or "" (goes into SENSOR_SUMMARY batch_hash). */
    fun shaFor(ctx: Context, w: Long): String = prefs(ctx).getString("sha_$w", "") ?: ""

    fun lastSizes(ctx: Context): String? = prefs(ctx).getString("last_sizes", null)

    fun pruneShas(ctx: Context, nowW: Long) {
        val p = prefs(ctx); val e = p.edit()
        p.all.keys.filter { it.startsWith("sha_") }.forEach { k -> if ((k.removePrefix("sha_").toLongOrNull() ?: 0) < nowW - 40 * 96) e.remove(k) }
        e.apply()
    }

    private fun gzip(b: ByteArray): ByteArray = ByteArrayOutputStream().also { o -> GZIPOutputStream(o).use { it.write(b) } }.toByteArray()

}

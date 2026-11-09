package network.hone.app.home

import android.content.Context
import android.location.Location
import android.net.wifi.WifiManager
import android.os.SystemClock
import android.util.Base64
import network.hone.app.sightings.SightingsEngine
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.SecureRandom

/**
 * Home on the phone: learns it from nights (GPS) and the home Wi-Fi, sets the home area once
 * ("hone_birds" home_lat/home_lon, the same prefs every privacy rule already reads), and answers
 * "is the phone at home right now?". Nothing here leaves the phone; the uploads only ever carry
 * the existing home_set flag. Wi-Fi networks are kept as salted hashes (HomeModel.wifiHashes).
 */
object HomeState {
    private const val SAMPLE_EVERY_MS = 10 * 60_000L
    private const val FIX_FRESH_MS = 15 * 60_000L
    private const val KEEP_MS = 21 * 86_400_000L
    private const val MAX_SAMPLES = 4000
    const val DEFAULT_RADIUS_M = 500f

    @Volatile private var lastSampleMs = 0L
    @Volatile private var wifiCache: Pair<Long, Boolean>? = null

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_home", Context.MODE_PRIVATE)
    private fun birds(ctx: Context) = ctx.getSharedPreferences("hone_birds", Context.MODE_PRIVATE)
    private fun samplesFile(ctx: Context) = File(ctx.filesDir, "hone-node/home_samples.jsonl")

    fun homeSet(ctx: Context) = birds(ctx).contains("home_lat")
    fun auto(ctx: Context) = homeSet(ctx) && prefs(ctx).getBoolean("auto", false)
    fun noticePending(ctx: Context) = homeSet(ctx) && prefs(ctx).getBoolean("notice", false)
    fun nights(ctx: Context) = prefs(ctx).getInt("nights", 0)
    fun homeWifiCount(ctx: Context) = homeWifi(ctx).size
    fun home(ctx: Context): Triple<Double, Double, Float>? = birds(ctx).takeIf { it.contains("home_lat") }?.let {
        Triple(it.getFloat("home_lat", 0f).toDouble(), it.getFloat("home_lon", 0f).toDouble(), it.getFloat("home_radius_m", DEFAULT_RADIUS_M))
    }

    private fun salt(ctx: Context): ByteArray {
        val p = prefs(ctx)
        p.getString("salt", null)?.let { return Base64.decode(it, Base64.NO_WRAP) }
        val s = ByteArray(16).also { SecureRandom().nextBytes(it) }
        p.edit().putString("salt", Base64.encodeToString(s, Base64.NO_WRAP)).apply()
        return s
    }

    private fun homeWifi(ctx: Context): Set<String> = prefs(ctx).getStringSet("wifi", emptySet()) ?: emptySet()

    /** Salted hashes of the Wi-Fi network this phone is connected to now (empty if none or no permission). */
    @Suppress("DEPRECATION")
    fun currentWifi(ctx: Context): Set<String> = runCatching {
        val wm = ctx.applicationContext.getSystemService(WifiManager::class.java) ?: return emptySet()
        val info = wm.connectionInfo ?: return emptySet()
        if (info.networkId == -1 && info.bssid == null) return emptySet()
        HomeModel.wifiHashes(salt(ctx), info.ssid, info.bssid)
    }.getOrDefault(emptySet())

    /** Connected to the remembered home Wi-Fi (checked at most every 30 s). */
    fun onHomeWifi(ctx: Context): Boolean {
        val now = SystemClock.elapsedRealtime()
        wifiCache?.let { (t, v) -> if (now - t < 30_000) return v }
        val home = homeWifi(ctx)
        val v = homeSet(ctx) && home.isNotEmpty() && HomeModel.onHomeWifi(currentWifi(ctx), home)
        wifiCache = now to v
        return v
    }

    /** At home: on the home Wi-Fi, or within the home radius (plus the fix's accuracy). */
    fun atHome(ctx: Context, lat: Double?, lon: Double?, acc: Float = 0f): Boolean {
        val h = home(ctx) ?: return false
        if (onHomeWifi(ctx)) return true
        if (lat == null || lon == null) return false
        return HomeModel.distM(lat, lon, h.first, h.second) <= h.third + acc
    }

    /**
     * The phone's fix; on the home Wi-Fi without a fresh fix, the home itself (accuracy = home
     * radius), so readings still count as at home rather than unlocated or at a stale place.
     */
    fun fixOrHome(ctx: Context): Location? {
        val fix = SightingsEngine.lastFix()
        val fresh = fix != null && (SystemClock.elapsedRealtimeNanos() - fix.elapsedRealtimeNanos) < FIX_FRESH_MS * 1_000_000L
        if (fresh) return fix
        val h = home(ctx)
        if (h != null && onHomeWifi(ctx)) return Location("home").apply {
            latitude = h.first; longitude = h.second; accuracy = h.third
            time = System.currentTimeMillis(); elapsedRealtimeNanos = SystemClock.elapsedRealtimeNanos()
        }
        return fix
    }

    /** Called from the service loop and the status worker: samples at night, then re-infers. */
    @Synchronized
    fun tick(ctx: Context) {
        val now = System.currentTimeMillis()
        if (now - lastSampleMs < SAMPLE_EVERY_MS) return
        lastSampleMs = now
        // Samples are taken at night only; the inference runs on every tick.
        val night = HomeModel.nightKey(now)
        val fix = SightingsEngine.lastFix()?.takeIf { (SystemClock.elapsedRealtimeNanos() - it.elapsedRealtimeNanos) < FIX_FRESH_MS * 1_000_000L }
        val wifi = if (night != null) currentWifi(ctx) else emptySet()
        if (night != null && (fix != null || wifi.isNotEmpty())) {
            val j = JSONObject().put("t", now).put("n", night).put("w", JSONArray(wifi.toList()))
            if (fix != null) j.put("lat", fix.latitude).put("lon", fix.longitude).put("acc", fix.accuracy.toDouble())
            runCatching { samplesFile(ctx).apply { parentFile?.mkdirs() }.appendText(j.toString() + "\n") }
        }
        if (samplesFile(ctx).exists()) runCatching { update(ctx, loadSamples(ctx, prune = true)) }
    }

    private fun loadSamples(ctx: Context, prune: Boolean): List<HomeModel.Sample> {
        val f = samplesFile(ctx); if (!f.exists()) return emptyList()
        val cut = System.currentTimeMillis() - KEEP_MS
        val lines = f.readLines().filter { it.isNotBlank() }
        val kept = lines.mapNotNull { l -> runCatching { JSONObject(l) }.getOrNull()?.takeIf { it.optLong("t") >= cut } }.takeLast(MAX_SAMPLES)
        if (prune && kept.size < lines.size) f.writeText(kept.joinToString("") { it.toString() + "\n" })
        return kept.map { o ->
            val w = o.optJSONArray("w"); val ws = HashSet<String>()
            if (w != null) for (i in 0 until w.length()) ws += w.getString(i)
            HomeModel.Sample(o.getLong("t"), o.optString("n").takeIf { it.isNotEmpty() },
                if (o.has("lat")) o.getDouble("lat") else null, if (o.has("lon")) o.getDouble("lon") else null,
                if (o.has("acc")) o.getDouble("acc").toFloat() else null, ws)
        }
    }

    private fun declined(ctx: Context): List<Pair<Double, Double>> = runCatching {
        val a = JSONArray(prefs(ctx).getString("declined", "[]"))
        (0 until a.length()).map { a.getJSONArray(it).let { p -> p.getDouble(0) to p.getDouble(1) } }
    }.getOrDefault(emptyList())

    /** Apply the inference: set an automatic home if none is saved; learn the home Wi-Fi either way. */
    fun update(ctx: Context, samples: List<HomeModel.Sample>) {
        val p = prefs(ctx)
        val cand = HomeModel.infer(samples, declined(ctx), p.getStringSet("declined_wifi", emptySet()) ?: emptySet()) ?: return
        val h = home(ctx)
        if (h == null) {
            val e = birds(ctx).edit().putFloat("home_lat", cand.lat.toFloat()).putFloat("home_lon", cand.lon.toFloat())
            if (!birds(ctx).contains("home_radius_m")) e.putFloat("home_radius_m", DEFAULT_RADIUS_M)
            e.apply()
            p.edit().putBoolean("auto", true).putBoolean("notice", true).putInt("nights", cand.nights)
                .putStringSet("wifi", cand.wifi).apply()
            wifiCache = null
        } else if (HomeModel.distM(cand.lat, cand.lon, h.first, h.second) <= h.third + HomeModel.RADIUS_M) {
            // Same place as the saved home (set by hand or earlier): remember its Wi-Fi too.
            p.edit().putStringSet("wifi", homeWifi(ctx) + cand.wifi).putInt("nights", cand.nights).apply()
            wifiCache = null
        }
    }

    /** One-time notice answered ("OK" or "Change"). */
    fun noticeSeen(ctx: Context) { prefs(ctx).edit().putBoolean("notice", false).apply() }

    /** Set home by hand at a fix (Settings). */
    fun setManual(ctx: Context, lat: Double, lon: Double) {
        val old = home(ctx)
        // Moving home a little keeps its Wi-Fi; moving it elsewhere forgets it.
        val keep = if (old != null && HomeModel.distM(lat, lon, old.first, old.second) <= old.third) homeWifi(ctx) else emptySet()
        birds(ctx).edit().putFloat("home_lat", lat.toFloat()).putFloat("home_lon", lon.toFloat()).apply()
        prefs(ctx).edit().putBoolean("auto", false).putBoolean("notice", false).putStringSet("wifi", keep).apply()
        wifiCache = null
    }

    /** "Not home" / Clear: forget the home and its Wi-Fi, and never suggest this place or network again. */
    fun decline(ctx: Context) {
        val p = prefs(ctx)
        home(ctx)?.let { h ->
            val a = runCatching { JSONArray(p.getString("declined", "[]")) }.getOrDefault(JSONArray())
            a.put(JSONArray().put(h.first).put(h.second))
            p.edit().putString("declined", a.toString()).apply()
        }
        p.edit().putStringSet("declined_wifi", (p.getStringSet("declined_wifi", emptySet()) ?: emptySet()) + homeWifi(ctx))
            .remove("wifi").putBoolean("auto", false).putBoolean("notice", false).apply()
        birds(ctx).edit().remove("home_lat").remove("home_lon").apply()
        wifiCache = null
    }

    /** Forget only the home Wi-Fi (Settings). */
    fun forgetWifi(ctx: Context) {
        val p = prefs(ctx)
        p.edit().putStringSet("declined_wifi", (p.getStringSet("declined_wifi", emptySet()) ?: emptySet()) + homeWifi(ctx)).remove("wifi").apply()
        wifiCache = null
    }
}

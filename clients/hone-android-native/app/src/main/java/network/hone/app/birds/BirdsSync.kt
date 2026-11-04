package network.hone.app.birds

import android.content.Context
import android.location.Location
import android.util.Base64
import android.util.Log
import network.hone.app.sightings.Cells
import network.hone.app.sound.Perch
import network.hone.app.telemetry.Canonical
import network.hone.app.telemetry.Telemetry
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.util.UUID

/**
 * Sends this phone's bird sightings and shared photos to the HONE birds indexer, and reads
 * everyone's sightings back. Rules (Josh, 2026-11-03):
 *  - inside the user's home area a sighting is published at H3 res 7 (~5 km);
 *    elsewhere at the finest cell the GPS fix supports (down to res 13, ~4 m edge);
 *  - every sighting/photo is signed: by the account's posting key when it is on the phone
 *    and matches the chain, otherwise by this device's seat key (the indexer holds those
 *    back until the seat is bound to the account);
 *  - photos only after the user agreed to UFL for that photo.
 * Public internet only (honemesh.net); queued windows flush when connectivity returns.
 */
object BirdsSync {
    private const val TAG = "HoneBirds"
    const val BASE = "https://honemesh.net/api/birds"
    private const val WINDOW_MS = 900_000L
    private const val PUBLISH_MIN_CONF = 70.0   // percent; or heard twice in the window

    /** HTTP failure the indexer gave a final answer to (4xx except 429): retrying will not help. */
    class Refused(val code: Int, msg: String) : Exception(msg)

    @Synchronized
    fun syncNow(ctx: Context) {
        runCatching { pushSightings(ctx) }.onFailure {
            Log.w(TAG, "sightings: ${it.message}"); Telemetry.upload(ctx, "birds", false, it.message)
        }
        runCatching { pushPhotos(ctx) }.onFailure {
            Log.w(TAG, "photos: ${it.message}"); Telemetry.upload(ctx, "photos", false, it.message)
        }
    }

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_birds", Context.MODE_PRIVATE)

    /** Account uploads are made for: sensor-node owner, else the seat account. */
    fun account(ctx: Context): String? = Telemetry.account(ctx)

    /** Cell to publish for a fix: ~5 km at home, otherwise as fine as the accuracy allows. */
    fun publicCell(ctx: Context, lat: Double, lon: Double, acc: Float): String? {
        val p = prefs(ctx)
        if (p.contains("home_lat")) {
            val d = FloatArray(1)
            Location.distanceBetween(lat, lon, p.getFloat("home_lat", 0f).toDouble(), p.getFloat("home_lon", 0f).toDouble(), d)
            if (d[0] <= p.getFloat("home_radius_m", 500f) + acc) return Cells.cell(lat, lon, 7)
        }
        val res = (13 downTo 5).firstOrNull { Cells.EDGE_M[it] >= acc } ?: return null
        return Cells.cell(lat, lon, res)
    }

    private fun pushSightings(ctx: Context) {
        val det = File(ctx.filesDir, "hone-node/sound/detections.jsonl")
        if (!det.exists()) { Telemetry.skipped(ctx, "birds", "no detections yet"); return }
        val current = System.currentTimeMillis() / WINDOW_MS
        // First run: start from the install of this feature (older plays/tests stay private).
        if (!prefs(ctx).contains("sent_window")) prefs(ctx).edit().putLong("sent_window", current - 1).apply()
        val last = prefs(ctx).getLong("sent_window", current - 1)
        if (last >= current - 1) return
        val acct = account(ctx) ?: run { Telemetry.skipped(ctx, "birds", "no account"); return }
        // window -> cell -> species -> (best conf, count)
        val byWin = sortedMapOf<Long, HashMap<String, HashMap<String, Pair<Double, Int>>>>()
        val rejected = Birds.labels(ctx).filterValues { !it }.keys
        var unlocated = 0
        det.forEachLine { l ->
            val j = runCatching { JSONObject(l) }.getOrNull() ?: return@forEachLine
            if ("${j.getLong("t")}|${j.getString("name")}" in rejected) return@forEachLine  // user said wrong
            val w = j.getLong("t") / WINDOW_MS
            if (w <= last || w >= current) return@forEachLine
            if (!j.has("lat")) { unlocated++; return@forEachLine }
            val cell = publicCell(ctx, j.getDouble("lat"), j.getDouble("lon"), j.optDouble("acc", 50.0).toFloat())
                ?: return@forEachLine
            // Out-of-range species go out only at 85%+ (flagged rare by the indexer).
            if (Range.current(ctx)?.contains(j.getString("name")) == false &&
                j.getDouble("conf") < Range.MIN_RARE * 100) return@forEachLine
            val sp = byWin.getOrPut(w) { HashMap() }.getOrPut(cell) { HashMap() }
            val prev = sp[j.getString("name")]
            sp[j.getString("name")] = Pair(maxOf(prev?.first ?: 0.0, j.getDouble("conf")), (prev?.second ?: 0) + 1)
        }
        if (unlocated > 0) Telemetry.skipped(ctx, "birds", "$unlocated detections without a location fix")
        val show = prefs(ctx).getBoolean("show_name", false)
        for ((w, cells) in byWin) {
            for ((cell, spAll) in cells) {
                // Publish only confident or repeated detections; weak one-offs stay private.
                val sp = spAll.filterValues { (conf, n) -> conf >= PUBLISH_MIN_CONF || n >= 2 }
                if (sp.isEmpty()) continue
                val payload = JSONObject().put("v", 1).put("sid", UUID.randomUUID().toString())
                    .put("window", w).put("cell", cell).put("model", Perch.MODEL_ID)
                    .put("species", JSONArray().apply {
                        sp.forEach { (name, v) ->
                            put(JSONObject().put("name", name).put("conf_q8", (v.first * 2.55).toInt().coerceIn(0, 255)).put("frames", v.second))
                        }
                    })
                payload.put("account", acct).put("show_name", show)
                try {
                    post("$BASE/sightings", signed(ctx, acct, payload.toString()))
                    Telemetry.upload(ctx, "birds", true)
                } catch (e: Refused) {
                    // A final 4xx for this window (bad cell, key refused...): record it and move on,
                    // so one bad window never blocks every later one.
                    Telemetry.upload(ctx, "birds", false, e.message)
                }
            }
            prefs(ctx).edit().putLong("sent_window", w).apply()
        }
        if (last < current - 1) prefs(ctx).edit().putLong("sent_window", current - 1).apply()
    }

    private fun pushPhotos(ctx: Context) {
        val root = File(ctx.filesDir, "birds/photos")
        val acct = account(ctx) ?: return
        val show = prefs(ctx).getBoolean("show_name", false)
        root.listFiles().orEmpty().forEach { dir ->
            dir.listFiles { f -> f.extension == "json" }.orEmpty().forEach { m ->
                val meta = JSONObject(m.readText())
                if (meta.optString("status") != "queued" || !meta.has("ufl_agreed_ms")) return@forEach
                val jpg = File(dir, m.nameWithoutExtension + ".jpg")
                val payload = JSONObject().put("v", 1).put("latin", meta.getString("latin"))
                    .put("t", meta.getLong("t")).put("license", "UFL")
                    .put("ufl_agreed_ms", meta.getLong("ufl_agreed_ms"))
                    .apply { meta.optJSONObject("evidence")?.let { put("evidence", it) } }
                payload.put("account", acct).put("show_name", show)
                val body = signed(ctx, acct, payload.toString())
                    .put("jpeg_b64", Base64.encodeToString(jpg.readBytes(), Base64.NO_WRAP))
                try {
                    val r = post("$BASE/photos", body)
                    meta.put("status", "published").put("id", r.optString("id"))
                    Telemetry.upload(ctx, "photos", true)
                } catch (e: Refused) {
                    meta.put("status", "refused").put("error", e.message?.take(160))
                    Telemetry.upload(ctx, "photos", false, e.message)
                }
                m.writeText(meta.toString())
            }
        }
    }

    /** True when the account's posting key is on this phone (UI hint; uploads also work with the seat key). */
    fun canSign(ctx: Context) = network.hone.app.sightings.RoleKeys.has(ctx, "posting")

    /** Signs the payload string (posting key, else seat key). Keys never leave the phone. */
    private fun signed(ctx: Context, account: String, payload: String): JSONObject {
        val s = Telemetry.signer(ctx, account)
        val env = Canonical.envelope(payload, s.keyType, s.seed)
        return JSONObject(env)
    }

    private fun post(url: String, body: JSONObject): JSONObject {
        val r = Telemetry.post(url, body.toString()) ?: error("offline")
        if (r.code == 429 || r.code >= 500) error("HTTP ${r.code} ${r.body.take(160)}")   // retry later
        if (r.code >= 400) throw Refused(r.code, "HTTP ${r.code} ${r.body.take(160)}")
        return runCatching { JSONObject(r.body) }.getOrDefault(JSONObject())
    }

    fun getJson(path: String, readTimeoutMs: Int = 20_000): String =
        (URL("$BASE$path").openConnection() as HttpURLConnection).apply {
            connectTimeout = 10_000; readTimeout = readTimeoutMs; setRequestProperty("User-Agent", "HONE-app/1.0")
        }.inputStream.bufferedReader().use { it.readText() }

    /** Download once, then serve from the phone (files/birds/cache). Photos are immutable by id. */
    fun cachedBytes(ctx: Context, url: String): ByteArray {
        val name = java.security.MessageDigest.getInstance("SHA-256").digest(url.toByteArray())
            .joinToString("") { "%02x".format(it) }.take(32)
        val f = File(ctx.filesDir, "birds/cache/$name").apply { parentFile?.mkdirs() }
        if (f.exists() && f.length() > 0) return f.readBytes()
        return getBytes(url).also { f.writeBytes(it) }
    }

    fun getBytes(url: String): ByteArray =
        (URL(if (url.startsWith("http")) url else "https://honemesh.net$url").openConnection() as HttpURLConnection).apply {
            connectTimeout = 10_000; readTimeout = 20_000
        }.inputStream.use { it.readBytes() }
}

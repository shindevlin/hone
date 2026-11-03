package network.hone.app.birds

import android.content.Context
import android.location.Location
import android.os.Build
import android.util.Base64
import android.util.Log
import network.hone.app.sightings.Cells
import network.hone.app.sound.Perch
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
 *  - anonymous by default: a fresh random id per sighting, no account, no signature;
 *    with "show my name" on, the account is named and the payload is signed by the device key;
 *  - photos only after the user agreed to UFL for that photo.
 */
object BirdsSync {
    private const val TAG = "HoneBirds"
    const val BASE = "https://honemesh.net/api/birds"
    private const val WINDOW_MS = 900_000L
    private const val PUBLISH_MIN_CONF = 50.0   // percent

    @Synchronized
    fun syncNow(ctx: Context) {
        runCatching { pushSightings(ctx) }.onFailure { Log.w(TAG, "sightings: ${it.message}") }
        runCatching { pushPhotos(ctx) }.onFailure { Log.w(TAG, "photos: ${it.message}") }
    }

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_birds", Context.MODE_PRIVATE)

    /** Owner account from the node settings. Uploads need it plus the imported posting key. */
    fun account(ctx: Context): String? {
        return runCatching {
            JSONObject(File(ctx.filesDir, "hone-node/sensor_settings.json").readText()).getString("owner")
        }.getOrNull()?.takeIf { it.isNotBlank() }
    }

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
        if (!det.exists()) return
        val current = System.currentTimeMillis() / WINDOW_MS
        // First run: start from the install of this feature (older plays/tests stay private).
        if (!prefs(ctx).contains("sent_window")) prefs(ctx).edit().putLong("sent_window", current - 1).apply()
        val last = prefs(ctx).getLong("sent_window", current - 1)
        // window -> cell -> species -> (best conf, count)
        val byWin = sortedMapOf<Long, HashMap<String, HashMap<String, Pair<Double, Int>>>>()
        val rejected = Birds.labels(ctx).filterValues { !it }.keys
        det.forEachLine { l ->
            val j = runCatching { JSONObject(l) }.getOrNull() ?: return@forEachLine
            if ("${j.getLong("t")}|${j.getString("name")}" in rejected) return@forEachLine  // user said wrong
            val w = j.getLong("t") / WINDOW_MS
            if (w <= last || w >= current || !j.has("lat")) return@forEachLine
            val cell = publicCell(ctx, j.getDouble("lat"), j.getDouble("lon"), j.optDouble("acc", 50.0).toFloat())
                ?: return@forEachLine
            // Out-of-range species go out only at 85%+ (flagged rare by the indexer).
            if (Range.current(ctx)?.contains(j.getString("name")) == false &&
                j.getDouble("conf") < Range.MIN_RARE * 100) return@forEachLine
            val sp = byWin.getOrPut(w) { HashMap() }.getOrPut(cell) { HashMap() }
            val prev = sp[j.getString("name")]
            sp[j.getString("name")] = Pair(maxOf(prev?.first ?: 0.0, j.getDouble("conf")), (prev?.second ?: 0) + 1)
        }
        val acct = account(ctx) ?: return
        if (!canSign(ctx)) return
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
                post(ctx, "$BASE/sightings", signed(ctx, payload.toString()))
            }
            prefs(ctx).edit().putLong("sent_window", w).apply()
        }
        if (byWin.isEmpty() && last < current - 1) prefs(ctx).edit().putLong("sent_window", current - 1).apply()
    }

    private fun pushPhotos(ctx: Context) {
        val root = File(ctx.filesDir, "birds/photos")
        val acct = account(ctx) ?: return
        if (!canSign(ctx)) return
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
                val body = signed(ctx, payload.toString())
                    .put("jpeg_b64", Base64.encodeToString(jpg.readBytes(), Base64.NO_WRAP))
                val r = post(ctx, "$BASE/photos", body)
                meta.put("status", "published").put("id", r.optString("id"))
                m.writeText(meta.toString())
            }
        }
    }

    fun canSign(ctx: Context) = network.hone.app.sightings.RoleKeys.has(ctx, "posting")

    /** Signs the payload with the account's posting key (Ed25519). The key never leaves the phone. */
    private fun signed(ctx: Context, payload: String): JSONObject {
        val seed = network.hone.app.sightings.RoleKeys.get(ctx, "posting") ?: error("posting key not imported")
        val sk = org.bouncycastle.crypto.params.Ed25519PrivateKeyParameters(seed, 0)
        val msg = payload.toByteArray()
        val sig = org.bouncycastle.crypto.signers.Ed25519Signer().run { init(true, sk); update(msg, 0, msg.size); generateSignature() }
        fun hex(b: ByteArray) = b.joinToString("") { "%02x".format(it) }
        return JSONObject().put("payload", payload).put("pub", hex(sk.generatePublicKey().encoded)).put("sig", hex(sig))
    }

    private fun post(ctx: Context, url: String, body: JSONObject): JSONObject {
        val c = (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"; doOutput = true; connectTimeout = 10_000; readTimeout = 30_000
            setRequestProperty("Content-Type", "application/json")
            setRequestProperty("User-Agent", "HONE-app/1.0")
        }
        c.outputStream.use { it.write(body.toString().toByteArray()) }
        val code = c.responseCode
        val text = (if (code < 400) c.inputStream else c.errorStream).bufferedReader().use { it.readText() }
        if (code >= 400) error("HTTP $code $text")
        return JSONObject(text)
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

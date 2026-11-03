package network.hone.app.birds

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLDecoder
import java.net.URLEncoder

/** One heard detection, kept only on this phone. */
data class Heard(val t: Long, val name: String, val conf: Double, val lat: Double?, val lon: Double?)

/** Everything this phone has heard of one species. */
data class MySpecies(
    val latin: String,
    val english: String?,
    val count: Int,
    val first: Long,
    val last: Long,
    val bestConf: Double,
    val history: List<Heard>,
)

/** A species picture from Wikimedia Commons, with its own license and credit. */
data class SpeciesImage(val file: File, val credit: String, val license: String, val pageUrl: String)

/** A photo this user submitted. Published under Hone's license (UFL) once uploaded. */
data class MyPhoto(val file: File, val latin: String, val t: Long, val status: String)

object Birds {
    @Volatile private var names: Map<String, String>? = null

    /** English common name from Wikidata (CC0), or null if Wikidata has none. */
    fun english(ctx: Context, latin: String): String? {
        val m = names ?: runCatching {
            val j = JSONObject(ctx.assets.open("birds/names_en.json").bufferedReader().readText())
            buildMap { j.keys().forEach { put(it, j.getString(it)) } }
        }.getOrDefault(emptyMap()).also { names = it }
        return m[latin]
    }

    private fun soundDir(ctx: Context) = File(ctx.filesDir, "hone-node/sound")
    private fun birdDir(ctx: Context) = File(ctx.filesDir, "birds").apply { mkdirs() }

    /** All detections on this phone, newest first. Older windows come from the window reports. */
    fun heard(ctx: Context): List<Heard> {
        val out = ArrayList<Heard>()
        val det = File(soundDir(ctx), "detections.jsonl")
        if (det.exists()) det.forEachLine { l ->
            runCatching {
                val j = JSONObject(l)
                out += Heard(j.getLong("t"), j.getString("name"), j.getDouble("conf"),
                    j.optDouble("lat").takeUnless { it.isNaN() }, j.optDouble("lon").takeUnless { it.isNaN() })
            }
        }
        val firstDet = out.minOfOrNull { it.t } ?: Long.MAX_VALUE
        val rep = File(soundDir(ctx), "reports.jsonl")
        if (rep.exists()) rep.forEachLine { l ->
            runCatching {
                val j = JSONObject(l)
                val t = j.getLong("window") * 900_000L
                if (t + 900_000L <= firstDet) {
                    val sp = j.getJSONArray("species")
                    for (i in 0 until sp.length()) {
                        val s = sp.getJSONObject(i)
                        out += Heard(t, s.getString("name"), s.getInt("conf_q8") * 100.0 / 255, null, null)
                    }
                }
            }
        }
        return out.sortedByDescending { it.t }
    }

    fun mySpecies(ctx: Context): List<MySpecies> =
        heard(ctx).groupBy { it.name }.map { (latin, hs) ->
            MySpecies(latin, english(ctx, latin), hs.size, hs.minOf { it.t }, hs.maxOf { it.t },
                hs.maxOf { it.conf }, hs)
        }.sortedByDescending { it.last }

    // ── species images (Wikimedia Commons, attributed) ─────────────────────────

    /** Cached image for a species, fetched once from Wikipedia/Commons. Call off the main thread. */
    fun image(ctx: Context, latin: String): SpeciesImage? {
        val dir = File(birdDir(ctx), "img").apply { mkdirs() }
        val key = latin.replace(' ', '_')
        val meta = File(dir, "$key.json")
        val img = File(dir, "$key.jpg")
        if (meta.exists()) {
            val j = JSONObject(meta.readText())
            return if (j.optBoolean("none") || !img.exists()) null
            else SpeciesImage(img, j.getString("credit"), j.getString("license"), j.getString("page"))
        }
        return runCatching {
            val sum = JSONObject(get("https://en.wikipedia.org/api/rest_v1/page/summary/" + enc(key)))
            val orig = sum.optJSONObject("originalimage")?.optString("source")
                ?: return@runCatching none(meta)
            val fileName = URLDecoder.decode(orig.substringAfterLast('/'), "UTF-8")
            val info = JSONObject(get(
                "https://commons.wikimedia.org/w/api.php?action=query&format=json&prop=imageinfo" +
                    "&iiprop=url|extmetadata&iiurlwidth=640&titles=" + enc("File:$fileName")
            )).getJSONObject("query").getJSONObject("pages")
            val ii = info.getJSONObject(info.keys().next()).getJSONArray("imageinfo").getJSONObject(0)
            val em = ii.getJSONObject("extmetadata")
            fun md(k: String) = em.optJSONObject(k)?.optString("value").orEmpty()
            val license = md("LicenseShortName").ifEmpty { "see source" }
            val artist = md("Artist").replace(Regex("<[^>]+>"), "").trim().ifEmpty { "Unknown" }
            val thumb = ii.optString("thumburl").ifEmpty { ii.getString("url") }
            img.writeBytes(getBytes(thumb))
            val page = ii.optString("descriptionurl")
            meta.writeText(JSONObject().put("credit", artist).put("license", license).put("page", page).toString())
            SpeciesImage(img, artist, license, page)
        }.getOrNull()
    }

    private fun none(meta: File): SpeciesImage? { meta.writeText("""{"none":true}"""); return null }

    fun bitmap(f: File, maxPx: Int = 640): Bitmap? = runCatching {
        val o = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(f.path, o)
        var s = 1; while (o.outWidth / (s * 2) >= maxPx) s *= 2
        BitmapFactory.decodeFile(f.path, BitmapFactory.Options().apply { inSampleSize = s })
    }.getOrNull()

    // ── user photos ─────────────────────────────────────────────────────────────

    /**
     * Saves a user's photo of [latin]. The image is re-encoded, which drops all EXIF data
     * (GPS position, device, time). Queued for upload to the Hone indexer under UFL.
     */
    fun addPhoto(ctx: Context, latin: String, uri: Uri, uflAgreedMs: Long): Boolean = runCatching {
        val src = ctx.contentResolver.openInputStream(uri)!!.use { BitmapFactory.decodeStream(it) } ?: return false
        val scale = minOf(1f, 2048f / maxOf(src.width, src.height))
        val bmp = if (scale < 1f) Bitmap.createScaledBitmap(src, (src.width * scale).toInt(), (src.height * scale).toInt(), true) else src
        val dir = File(birdDir(ctx), "photos/" + latin.replace(' ', '_')).apply { mkdirs() }
        val t = System.currentTimeMillis()
        File(dir, "$t.jpg").outputStream().use { bmp.compress(Bitmap.CompressFormat.JPEG, 88, it) }
        File(dir, "$t.json").writeText(JSONObject().put("latin", latin).put("t", t)
            .put("license", "UFL").put("ufl_agreed_ms", uflAgreedMs).put("status", "queued").toString())
        true
    }.getOrDefault(false)

    fun myPhotos(ctx: Context, latin: String): List<MyPhoto> {
        val dir = File(birdDir(ctx), "photos/" + latin.replace(' ', '_'))
        return dir.listFiles { f -> f.extension == "jpg" }.orEmpty().mapNotNull { f ->
            val m = File(dir, f.nameWithoutExtension + ".json")
            val j = runCatching { JSONObject(m.readText()) }.getOrNull() ?: return@mapNotNull null
            MyPhoto(f, latin, j.getLong("t"), j.optString("status", "queued"))
        }.sortedByDescending { it.t }
    }

    // ── http ────────────────────────────────────────────────────────────────────

    private fun enc(s: String) = URLEncoder.encode(s, "UTF-8").replace("+", "%20")
    private fun conn(url: String) = (URL(url).openConnection() as HttpURLConnection).apply {
        connectTimeout = 10_000; readTimeout = 15_000
        setRequestProperty("User-Agent", "HONE-app/1.0 (https://honemesh.net)")
    }
    private fun get(url: String): String = conn(url).inputStream.bufferedReader().use { it.readText() }
    private fun getBytes(url: String): ByteArray = conn(url).inputStream.use { it.readBytes() }
}

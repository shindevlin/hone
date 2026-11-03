package network.hone.app.birds

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.util.Log
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLDecoder
import java.net.URLEncoder

/** One heard detection, kept only on this phone. */
data class Heard(val t: Long, val name: String, val conf: Double, val lat: Double?, val lon: Double?, val rare: Boolean = false)

/** Everything this phone has heard of one species. */
data class MySpecies(
    val latin: String,
    val english: String?,
    val count: Int,
    val first: Long,
    val last: Long,
    val bestConf: Double,
    val history: List<Heard>,
    /** Not on the local species list: kept only when heard at 85%+. */
    val rare: Boolean = false,
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
                    j.optDouble("lat").takeUnless { it.isNaN() }, j.optDouble("lon").takeUnless { it.isNaN() },
                    j.optBoolean("rare"))
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

    /** Species heard here. Once the local range is known, species that don't occur here are left out. */
    fun mySpecies(ctx: Context): List<MySpecies> {
        val range = Range.current(ctx)
        val keep = heard(ctx).filter { range == null || it.name in range || it.conf >= Range.MIN_RARE * 100 }
        return keep.groupBy { it.name }.map { (latin, hs) ->
            MySpecies(latin, english(ctx, latin), hs.size, hs.minOf { it.t }, hs.maxOf { it.t },
                hs.maxOf { it.conf }, hs, rare = range != null && latin !in range)
        }.sortedByDescending { it.last }
    }

    // ── species images (Wikimedia Commons, attributed) ─────────────────────────

    private val imgLock = Any()

    /**
     * Picture for a species, call off the main thread. Order: the user's own photo, a HONE
     * community photo (UFL), the Wikipedia lead image, then Wikidata's image (P18), both from
     * Wikimedia Commons with credit and license. One network fetch at a time.
     */
    fun image(ctx: Context, latin: String): SpeciesImage? {
        myPhotos(ctx, latin).firstOrNull()?.let { return SpeciesImage(it.file, "you", "UFL", "") }
        val dir = File(birdDir(ctx), "img").apply { mkdirs() }
        val key = latin.replace(' ', '_')
        val meta = File(dir, "$key.json")
        val img = File(dir, "$key.jpg")
        fun cached(): SpeciesImage? {
            if (!meta.exists()) return null
            val j = JSONObject(meta.readText())
            if (j.optBoolean("none")) return if (System.currentTimeMillis() - meta.lastModified() < 86_400_000L) NONE else null
            return if (img.exists()) SpeciesImage(img, j.getString("credit"), j.getString("license"), j.getString("page")) else null
        }
        cached()?.let { return it.takeUnless { it === NONE } }
        synchronized(imgLock) {
            cached()?.let { return it.takeUnless { it === NONE } }
            val got = runCatching { community(latin, img) }.onFailure { Log.w(TAG, "community img $latin: ${it.message}") }.getOrNull()
                ?: runCatching { commonsFile(wikipediaFile(key), img) }.onFailure { Log.w(TAG, "wikipedia img $latin: ${it.message}") }.getOrNull()
                ?: runCatching { commonsFile(wikidataFile(latin), img) }.onFailure { Log.w(TAG, "wikidata img $latin: ${it.message}") }.getOrNull()
            if (got == null) { meta.writeText("""{"none":true}"""); return null }
            meta.writeText(JSONObject().put("credit", got.credit).put("license", got.license).put("page", got.pageUrl).toString())
            return got
        }
    }

    private val NONE = SpeciesImage(File(""), "", "", "")
    private const val TAG = "HoneBirds"

    private fun community(latin: String, out: File): SpeciesImage? {
        val p = JSONObject(BirdsSync.getJson("/species/" + enc(latin))).optJSONArray("photos") ?: return null
        if (p.length() == 0) return null
        val first = p.getJSONObject(0)
        out.writeBytes(BirdsSync.getBytes(first.getString("url")))
        return SpeciesImage(out, first.optString("account").ifEmpty { "HONE community" }, "UFL", "")
    }

    private fun wikipediaFile(key: String): String? {
        val sum = JSONObject(get("https://en.wikipedia.org/api/rest_v1/page/summary/" + enc(key)))
        val orig = sum.optJSONObject("originalimage")?.optString("source") ?: return null
        return URLDecoder.decode(orig.substringBefore("?").substringAfterLast("/"), "UTF-8")
    }

    private fun wikidataFile(latin: String): String? {
        val q = "SELECT ?img WHERE { ?i wdt:P225 \"$latin\" ; wdt:P18 ?img } LIMIT 1"
        val r = JSONObject(get("https://query.wikidata.org/sparql?format=json&query=" + enc(q)))
        val b = r.getJSONObject("results").getJSONArray("bindings")
        if (b.length() == 0) return null
        val url = b.getJSONObject(0).getJSONObject("img").getString("value")
        return URLDecoder.decode(url.substringAfterLast("/"), "UTF-8")
    }

    private fun commonsFile(fileName: String?, out: File): SpeciesImage? {
        if (fileName.isNullOrBlank()) return null
        val info = JSONObject(get(
            "https://commons.wikimedia.org/w/api.php?action=query&format=json&prop=imageinfo" +
                "&iiprop=url|extmetadata&iiurlwidth=640&titles=" + enc("File:$fileName")
        )).getJSONObject("query").getJSONObject("pages")
        val ii = info.getJSONObject(info.keys().next()).getJSONArray("imageinfo").getJSONObject(0)
        val em = ii.getJSONObject("extmetadata")
        fun md(k: String) = em.optJSONObject(k)?.optString("value").orEmpty()
        val license = md("LicenseShortName").ifEmpty { "see source" }
        val artist = md("Artist").replace(Regex("<[^>]+>"), "").trim().ifEmpty { "Unknown" }
        out.writeBytes(getBytes(ii.optString("thumburl").ifEmpty { ii.getString("url") }))
        return SpeciesImage(out, artist, license, ii.optString("descriptionurl"))
    }

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

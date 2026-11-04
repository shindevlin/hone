package network.hone.app.birds

import android.content.Context
import android.net.Uri
import androidx.media3.common.util.UnstableApi
import androidx.media3.database.StandaloneDatabaseProvider
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.datasource.cache.CacheDataSource
import androidx.media3.datasource.cache.LeastRecentlyUsedCacheEvictor
import androidx.media3.datasource.cache.SimpleCache
import org.json.JSONObject
import java.io.File

/**
 * Reference calls from xeno-canto, for listening only. The indexer returns metadata
 * (GET /api/birds/call/<latin>); audio streams from xeno-canto's own file URL, played
 * unmodified. These recordings never feed rewards, scoring, training data, anti-gaming
 * or marketing: nothing here touches Birds.label, sightings or the sound pipeline.
 */
data class XcRecording(
    val xcId: String, val recordist: String, val license: String, val licenseUrl: String,
    val quality: String, val fileUrl: String, val pageUrl: String, val length: String, val type: String,
) {
    val credit get() = "Recording: $recordist, XC$xcId, $license"
}

sealed class CallResult {
    data class Ok(val recordings: List<XcRecording>) : CallResult()
    data class Unavailable(val reason: String) : CallResult()
}

@UnstableApi
object BirdCalls {
    private const val MAX_FILES = 20
    private const val MAX_AGE_MS = 30L * 86_400_000
    private const val MAX_BYTES = 120L * 1024 * 1024
    private val meta = HashMap<String, CallResult.Ok>()
    @Volatile private var cache: SimpleCache? = null

    fun fetch(latin: String): CallResult {
        synchronized(meta) { meta[latin] }?.let { return it }
        return runCatching {
            val c = java.net.URL("${BirdsSync.BASE}/call/" + Uri.encode(latin)).openConnection() as java.net.HttpURLConnection
            c.connectTimeout = 10_000; c.readTimeout = 40_000; c.setRequestProperty("User-Agent", "HONE-app/1.0")
            val code = c.responseCode
            if (code == 503 || code == 404) return CallResult.Unavailable("reference calls are not available yet")
            if (code >= 400) return CallResult.Unavailable("could not load calls (HTTP $code)")
            val o = JSONObject(c.inputStream.bufferedReader().use { it.readText() })
            val arr = o.optJSONArray("recordings")
            val list = (0 until (arr?.length() ?: 0)).map { arr!!.getJSONObject(it) }.mapNotNull { r ->
                val file = r.optString("file_url")
                if (!file.startsWith("https://")) null else XcRecording(
                    r.optString("xc_id"), r.optString("recordist"), r.optString("license"),
                    r.optString("license_url"), r.optString("quality"), file, r.optString("page_url"),
                    r.optString("length"), r.optString("type"))
            }
            if (list.isEmpty()) CallResult.Unavailable("no reference recordings for this species")
            else CallResult.Ok(list).also { synchronized(meta) { meta[latin] = it } }
        }.getOrElse { CallResult.Unavailable("could not load calls (offline?)") }
    }

    @Synchronized
    private fun cache(ctx: Context): SimpleCache = cache ?: SimpleCache(
        File(ctx.applicationContext.cacheDir, "bird-calls"),
        LeastRecentlyUsedCacheEvictor(MAX_BYTES),
        StandaloneDatabaseProvider(ctx.applicationContext),
    ).also { cache = it }

    /** Stream from xeno-canto, keeping a phone copy: at most [MAX_FILES], none older than 30 days. */
    fun dataSource(ctx: Context): CacheDataSource.Factory {
        prune(ctx)
        val http = DefaultHttpDataSource.Factory()
            .setUserAgent("HONE-app/1.0 (https://honemesh.net)")
            .setAllowCrossProtocolRedirects(true)
            .setConnectTimeoutMs(10_000).setReadTimeoutMs(20_000)
        return CacheDataSource.Factory().setCache(cache(ctx)).setUpstreamDataSourceFactory(http)
            .setFlags(CacheDataSource.FLAG_IGNORE_CACHE_ON_ERROR)
    }

    fun cacheKey(r: XcRecording) = "xc-${r.xcId}"

    private fun prune(ctx: Context) = runCatching {
        val c = cache(ctx)
        val now = System.currentTimeMillis()
        val touched = c.keys.associateWith { k -> c.getCachedSpans(k).maxOfOrNull { it.lastTouchTimestamp } ?: 0L }
        touched.filter { now - it.value > MAX_AGE_MS }.keys.forEach { c.removeResource(it) }
        touched.filter { now - it.value <= MAX_AGE_MS }.entries.sortedByDescending { it.value }
            .drop(MAX_FILES - 1).forEach { c.removeResource(it.key) }   // room for the next one
    }
}
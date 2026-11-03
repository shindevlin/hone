package network.hone.app.birds

import android.content.Context
import android.util.Log
import network.hone.app.sightings.Cells
import network.hone.app.sightings.SightingsEngine
import org.json.JSONObject
import java.io.File

/**
 * Species recorded near the phone (GBIF, via the HONE birds indexer, H3 res-3 area, 250 km).
 * Perch scores ~15,000 species worldwide; only species that occur here may count as heard.
 * If the list isn't known yet, a much higher confidence bar applies instead.
 */
object Range {
    const val MIN_IN_RANGE = 0.30f
    const val MIN_UNKNOWN = 0.60f
    private const val TTL_MS = 30L * 86_400_000L

    @Volatile private var cell: String? = null
    @Volatile private var names: Set<String>? = null
    @Volatile private var fetching: String? = null

    /** True when a Perch score for [latin] should count as a detection here. */
    fun allowed(ctx: Context, latin: String, p: Float): Boolean {
        val set = current(ctx)
        return if (set != null) latin in set && p >= MIN_IN_RANGE else p >= MIN_UNKNOWN
    }

    /** The species set for the phone's current area, or null if not loaded yet. */
    fun current(ctx: Context): Set<String>? {
        val fix = SightingsEngine.lastFix() ?: return names
        val c = Cells.cell(fix.latitude, fix.longitude, 3) ?: return names
        if (c != cell) {
            cell = c
            names = load(ctx, c)
            if (names == null) fetch(ctx.applicationContext, c)
        }
        return names
    }

    private fun file(ctx: Context, c: String) = File(ctx.filesDir, "birds/range_$c.json")

    private fun load(ctx: Context, c: String): Set<String>? = runCatching {
        val f = file(ctx, c)
        if (!f.exists() || System.currentTimeMillis() - f.lastModified() > TTL_MS) return null
        val a = JSONObject(f.readText()).getJSONArray("species")
        (0 until a.length()).map { a.getString(it) }.toSet()
    }.getOrNull()

    private fun fetch(ctx: Context, c: String) {
        if (fetching == c) return
        fetching = c
        Thread {
            runCatching {
                val txt = BirdsSync.getJson("/range?cell=$c", readTimeoutMs = 600_000)
                file(ctx, c).apply { parentFile?.mkdirs() }.writeText(txt)
                if (cell == c) names = load(ctx, c)
                Log.i("HoneBirds", "range $c: ${names?.size} species")
            }.onFailure { Log.w("HoneBirds", "range $c failed: ${it.message}") }
            fetching = null
        }.start()
    }
}

package network.hone.app.birds

import android.content.Context
import android.location.Location
import network.hone.app.sightings.Cells
import network.hone.app.sightings.SightingsEngine
import java.util.zip.GZIPInputStream
import kotlin.math.roundToInt

/** One place (H3 res-9 cell) where this phone heard a species. Built from the local log only. */
data class HeardPlace(
    /** H3 res-9 cells of this place (one, or every cell inside the home area). */
    val cells: List<String>,
    val lat: Double,
    val lon: Double,
    val count: Int,
    val first: Long,
    val last: Long,
    val bestConf: Double,
    val label: String,
)

data class HeardPlaces(val places: List<HeardPlace>, val unlocated: Int)

/**
 * "Where I heard this bird". Everything here runs on the phone: the detection log, the home area
 * and an offline place list (GeoNames cities1000, assets/places). Nothing is sent anywhere.
 */
object Places {
    const val RES = 9
    private const val HOME = "\u0000home"
    private const val NAME_MAX_M = 4000f

    fun forSpecies(ctx: Context, history: List<Heard>): HeardPlaces {
        val located = history.filter { it.lat != null && it.lon != null }
        val home = home(ctx)
        val here = SightingsEngine.lastFix()?.let { it.latitude to it.longitude }
        // Res-9 cells; everything inside the home area is one place, "Home".
        fun cellOf(h: Heard) = Cells.cell(h.lat!!, h.lon!!, RES)
            ?: "%.3f,%.3f".format(Math.round(h.lat * 300) / 300.0, Math.round(h.lon * 300) / 300.0)
        fun atHome(h: Heard) = home != null && dist(h.lat!!, h.lon!!, home.lat, home.lon) <= home.radius
        val groups = located.groupBy { if (atHome(it)) HOME else cellOf(it) }
        val places = groups.map { (key, hs) ->
            val lat = hs.map { it.lat!! }.average(); val lon = hs.map { it.lon!! }.average()
            HeardPlace(hs.map { cellOf(it) }.distinct(), lat, lon, hs.size, hs.minOf { it.t }, hs.maxOf { it.t },
                hs.maxOf { it.conf }, if (key == HOME) "Home" else label(ctx, lat, lon, home, here))
        }.sortedWith(compareByDescending<HeardPlace> { it.label == "Home" }.thenByDescending { it.count }.thenByDescending { it.last })
        return HeardPlaces(places, history.size - located.size)
    }

    private data class Home(val lat: Double, val lon: Double, val radius: Float)

    private fun home(ctx: Context): Home? {
        val p = ctx.getSharedPreferences("hone_birds", Context.MODE_PRIVATE)
        if (!p.contains("home_lat")) return null
        return Home(p.getFloat("home_lat", 0f).toDouble(), p.getFloat("home_lon", 0f).toDouble(), p.getFloat("home_radius_m", 500f))
    }

    /** "Home" inside the home area, else the nearest named place, else distance and direction. */
    private fun label(ctx: Context, lat: Double, lon: Double, home: Home?, here: Pair<Double, Double>?): String {
        if (home != null && dist(lat, lon, home.lat, home.lon) <= home.radius) return "Home"
        nearestName(ctx, lat, lon)?.let { return it }
        val ref = home?.let { it.lat to it.lon } ?: here
        if (ref != null) {
            val d = dist(ref.first, ref.second, lat, lon)
            val from = if (home != null) "from home" else "from here"
            return "${km(d)} ${compass(bearing(ref.first, ref.second, lat, lon))} $from"
        }
        return "%.2f, %.2f".format(lat, lon)
    }

    fun km(m: Float): String = if (m < 1000) "${(m / 10).roundToInt() * 10} m" else "%.1f km".format(m / 1000)

    fun compass(b: Float): String {
        val dirs = arrayOf("N", "NE", "E", "SE", "S", "SW", "W", "NW")
        return dirs[((((b % 360) + 360) % 360) / 45f).roundToInt() % 8]
    }

    private fun dist(a: Double, b: Double, c: Double, d: Double): Float {
        val r = FloatArray(1); Location.distanceBetween(a, b, c, d, r); return r[0]
    }

    private fun bearing(a: Double, b: Double, c: Double, d: Double): Float {
        val r = FloatArray(2); Location.distanceBetween(a, b, c, d, r); return r[1]
    }

    // ── offline gazetteer, sorted by latitude ──────────────────────────────────
    private class Gaz(val lat: FloatArray, val lon: FloatArray, val name: Array<String>)
    @Volatile private var gaz: Gaz? = null

    private fun gaz(ctx: Context): Gaz? = gaz ?: synchronized(this) {
        gaz ?: runCatching {
            val la = ArrayList<Float>(180_000); val lo = ArrayList<Float>(180_000); val nm = ArrayList<String>(180_000)
            // The build unpacks places.tsv.gz into assets/places/places.tsv (stored compressed in the APK).
            val input = runCatching { ctx.assets.open("places/places.tsv") }.getOrNull()
                ?: GZIPInputStream(ctx.assets.open("places/places.tsv.gz"))
            input.bufferedReader().forEachLine { l ->
                val f = l.split('\t')
                if (f.size >= 3) { nm += f[0]; la += f[1].toFloat(); lo += f[2].toFloat() }
            }
            Gaz(la.toFloatArray(), lo.toFloatArray(), nm.toTypedArray())
        }.getOrNull().also { gaz = it }
    }

    /** Nearest populated place within [NAME_MAX_M], e.g. "Antiguo Cuscatlán", or "near X" past 1.5 km. */
    fun nearestName(ctx: Context, lat: Double, lon: Double): String? {
        val g = gaz(ctx) ?: return null
        val dLat = 0.04f
        var lo = 0; var hi = g.lat.size
        while (lo < hi) { val m = (lo + hi) ushr 1; if (g.lat[m] < lat - dLat) lo = m + 1 else hi = m }
        var best = -1; var bestD = Float.MAX_VALUE
        var i = lo
        while (i < g.lat.size && g.lat[i] <= lat + dLat) {
            if (kotlin.math.abs(g.lon[i] - lon) < 0.05) {
                val d = dist(lat, lon, g.lat[i].toDouble(), g.lon[i].toDouble())
                if (d < bestD) { bestD = d; best = i }
            }
            i++
        }
        if (best < 0 || bestD > NAME_MAX_M) return null
        return if (bestD > 1500) "near ${g.name[best]}" else g.name[best]
    }
}

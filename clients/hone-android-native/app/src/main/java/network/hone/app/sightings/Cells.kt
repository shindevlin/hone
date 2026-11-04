package network.hone.app.sightings

import android.util.Log
import com.uber.h3core.H3Core

/**
 * H3 cells. A reading goes to the finest resolution whose average edge is at least its
 * accuracy radius (res 11 for ~25 m fixes, res 9 for ~200 m, down to res 5 for ~10 km),
 * so a poor fix still counts, just in a coarser cell. The private layer always keeps a
 * res-11 cell of the best estimate. h3-java's android JNI build is packaged by
 * app/build.gradle.kts; if it fails to load, readings stay unlocated.
 */
object Cells {
    const val PUBLIC_RES = 9
    const val PRIVATE_RES = 11
    const val COARSEST = 5

    /** H3 average hexagon edge length in metres, res 0..15. */
    val EDGE_M = doubleArrayOf(1281256.0, 483056.8, 182512.9, 68979.2, 26071.8, 9854.1, 3724.5,
        1406.5, 531.4, 200.8, 75.9, 28.7, 10.8, 4.1, 1.5, 0.58)

    private val h3: H3Core? by lazy {
        runCatching { H3Core.newSystemInstance() }
            .onFailure { Log.e("HoneSight", "h3 native load failed", it) }
            .getOrNull()
    }

    fun available(): Boolean = h3 != null

    fun cell(lat: Double, lon: Double, res: Int): String? =
        runCatching { h3?.latLngToCellAddress(lat, lon, res) }.getOrNull()

    /** Finest res (<= PRIVATE_RES) whose edge covers [accM]; null if worse than res COARSEST. */
    fun resFor(accM: Float): Int? = (PRIVATE_RES downTo COARSEST).firstOrNull { EDGE_M[it] >= accM }

    fun parent(cell: String, res: Int): String? =
        runCatching { h3?.cellToParentAddress(cell, res) }.getOrNull()

    fun res(cell: String): Int = runCatching { h3?.getResolution(cell) }.getOrNull() ?: -1

    /** Hexagon corners as (lat, lng). */
    fun boundary(cell: String): List<Pair<Double, Double>>? =
        runCatching { h3?.cellToBoundary(cell)?.map { it.lat to it.lng } }.getOrNull()

    fun center(cell: String): Pair<Double, Double>? =
        runCatching { h3?.cellToLatLng(cell)?.let { it.lat to it.lng } }.getOrNull()
}

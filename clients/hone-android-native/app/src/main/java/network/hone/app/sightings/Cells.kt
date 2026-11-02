package network.hone.app.sightings

import android.util.Log
import com.uber.h3core.H3Core

/**
 * H3 cells. Public summaries use res 9 (~170 m); the private layer uses res 11 (~25 m),
 * which matches phone GPS error (~10 m) without pretending to more precision.
 * h3-java 4.5.x ships an android-arm64 JNI build (packaged by app/build.gradle.kts); if it fails to load, cells are null and
 * every reading is kept un-located (no summary is queued for submission without a cell).
 */
object Cells {
    const val PUBLIC_RES = 9
    const val PRIVATE_RES = 11

    private val h3: H3Core? by lazy {
        runCatching { H3Core.newSystemInstance() }
            .onFailure { Log.e("HoneSight", "h3 native load failed", it) }
            .getOrNull()
    }

    fun available(): Boolean = h3 != null

    fun cell(lat: Double, lon: Double, res: Int): String? =
        runCatching { h3?.latLngToCellAddress(lat, lon, res) }.getOrNull()
}

package network.hone.app.raw

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.location.Location
import android.location.LocationManager
import android.os.SystemClock
import network.hone.app.sightings.Cells

/**
 * Every reading carries the device's own coarse area (Josh 2026-11-10 10:52), H3 res 7-8, best
 * source first: 1 GPS/fused fix, 2 network (cell towers, coarse location, no GPS), 3 Wi-Fi
 * fingerprint match against cells this device verified, 4 the last verified cell (with its age),
 * 5 IP geolocation done on the ingest (src "none" here). Privacy: res 7 at/near home or with no
 * home known, otherwise res 8 (public summaries may still go to res 9 under the summary rule).
 */
object AreaLocator {
    data class Area(val cell: String?, val res: Int?, val src: String, val accM: Int?, val inferred: Boolean, val ageS: Long?) {
        fun toMap(): Map<String, Any?> = mapOf("cell" to cell, "res" to res, "src" to src, "acc_m" to accM, "inferred" to inferred, "age_s" to ageS)
        /** Integer metric code for SENSOR_SUMMARY (metrics are integers): 1 gps/fused, 2 network, 3 wifi_fp, 4 last, 5 ip, 0 none. */
        val srcCode: Int get() = when (src) { "gps", "fused" -> 1; "network" -> 2; "wifi_fp" -> 3; "last_verified" -> 4; "ip" -> 5; else -> 0 }
    }

    private const val LIVE_MS = 10 * 60_000L
    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_area", Context.MODE_PRIVATE)
    private fun ageMs(l: Location) = (SystemClock.elapsedRealtimeNanos() - l.elapsedRealtimeNanos) / 1_000_000

    /** res 7 at/near home or with no home known; else 8. */
    fun areaRes(ctx: Context, lat: Double, lon: Double): Int {
        val h = runCatching { network.hone.app.home.HomeState.home(ctx) }.getOrNull() ?: return 7
        val d = FloatArray(1)
        Location.distanceBetween(lat, lon, h.first, h.second, d)
        return if (d[0] <= h.third + Cells.EDGE_M[7].toFloat()) 7 else 8
    }

    private fun srcName(provider: String?): String = when {
        provider == null -> "fused"
        provider.contains("gps") -> "gps"
        provider.contains("fused") -> "fused"
        provider.contains("network") -> "network"
        provider.contains("wifi-fp") -> "wifi_fp"
        else -> "fused"
    }

    /** [fix]: the sightings engine's best current fix and its source label, if any. */
    @SuppressLint("MissingPermission")
    fun current(ctx: Context, fix: Pair<Location, String>?): Area {
        // 1-3: a live fix (GPS/fused, or the engine's Wi-Fi fingerprint fix), then network location.
        val live = fix?.takeIf { ageMs(it.first) in 0..LIVE_MS }
            ?: runCatching {
                val coarse = ctx.checkSelfPermission(Manifest.permission.ACCESS_COARSE_LOCATION) == PackageManager.PERMISSION_GRANTED
                if (!coarse) null else ctx.getSystemService(LocationManager::class.java)
                    ?.getLastKnownLocation(LocationManager.NETWORK_PROVIDER)?.takeIf { ageMs(it) in 0..LIVE_MS }?.let { it to "network" }
            }.getOrNull()
        if (live != null) {
            val (l, label) = live
            val res = areaRes(ctx, l.latitude, l.longitude)
            val cell = Cells.cell(l.latitude, l.longitude, res)
            if (cell != null) {
                prefs(ctx).edit().putString("cell7", Cells.cell(l.latitude, l.longitude, 7)).putString("cell", cell)
                    .putInt("res", res).putLong("ms", System.currentTimeMillis()).apply()
                return Area(cell, res, srcName(label), l.accuracy.toInt(), false, ageMs(l) / 1000)
            }
        }
        // 4: the last verified cell, with its age.
        val p = prefs(ctx)
        val last = p.getString("cell", null)
        if (last != null) return Area(last, p.getInt("res", 7), "last_verified", null, true,
            (System.currentTimeMillis() - p.getLong("ms", 0)) / 1000)
        // 5: none here; the ingest may add an IP-level area.
        return Area(null, null, "none", null, true, null)
    }

    /** The res-7 cell of the last verified area (token salt for identifiers), or "none". */
    fun salt(ctx: Context): String = prefs(ctx).getString("cell7", null) ?: "none"
}

package network.hone.app.sightings

import kotlin.math.asin
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.min
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * Movement register from location fixes alone. Replaces step counting (dropped 2026-11-09):
 * no "Physical activity" permission, no step counts, no health data.
 *
 * Two sources, best first:
 * 1. GNSS Doppler speed, when a fix carries a speed with a known accuracy of <= 1 m/s
 *    (or, without an accuracy figure, a GNSS-grade fix of <= 20 m).
 * 2. Displacement between fixes, judged against both fixes' accuracy. Moved for sure:
 *    the distance is more than twice their combined accuracy. Still for sure: enough time
 *    has passed that a walker would have crossed that margin and the phone didn't.
 *    Otherwise the interval is left unclassified, so a poor fix never invents movement.
 *
 * Classes: still (< 0.4 m/s), walking (< 2.8 m/s, incl. a slow birding pace and jogging),
 * fast (bike, run or vehicle). Per window it reports the seconds in each class and the
 * state with the most time, or "unknown" with less than 2 minutes classified.
 * Pure Kotlin, so it is unit tested without a phone.
 */
class Movement {
    data class Fix(
        val tMs: Long,                 // monotonic (elapsedRealtime) milliseconds
        val lat: Double,
        val lon: Double,
        val accM: Float,               // horizontal accuracy radius
        val speedMps: Float? = null,   // GNSS Doppler speed, if the fix has one
        val speedAccMps: Float? = null,
    )

    enum class State(val id: String) { STILL("still"), WALKING("walking"), FAST("fast"), UNKNOWN("unknown") }

    data class Summary(val state: State, val stillS: Int, val walkS: Int, val fastS: Int, val via: String)

    companion object {
        const val STILL_MAX_MPS = 0.4
        const val WALK_MAX_MPS = 2.8
        const val MAX_ACC_M = 100f
        const val MAX_GAP_MS = 180_000L          // longer gaps aren't credited to any class
        const val MAX_DOPPLER_CREDIT_MS = 60_000L
        const val DOPPLER_ACC_MAX_MPS = 1.0f
        const val MIN_CLASSIFIED_S = 120

        fun distM(a: Fix, b: Fix): Double {
            val r = 6_371_008.8
            val p1 = Math.toRadians(a.lat); val p2 = Math.toRadians(b.lat)
            val dp = p2 - p1; val dl = Math.toRadians(b.lon - a.lon)
            val h = sin(dp / 2) * sin(dp / 2) + cos(p1) * cos(p2) * sin(dl / 2) * sin(dl / 2)
            return 2 * r * asin(min(1.0, sqrt(h)))
        }
    }

    private var lastT: Long? = null
    private var anchor: Fix? = null
    private var stillMs = 0L
    private var walkMs = 0L
    private var fastMs = 0L
    private var viaDoppler = 0
    private var viaDisp = 0

    private fun dopplerOk(f: Fix): Boolean {
        val v = f.speedMps ?: return false
        if (!v.isFinite() || v < 0f) return false
        val acc = f.speedAccMps
        return if (acc != null && acc.isFinite() && acc > 0f) acc <= DOPPLER_ACC_MAX_MPS else f.accM <= 20f
    }

    private fun credit(speedMps: Double, ms: Long) {
        when {
            speedMps < STILL_MAX_MPS -> stillMs += ms
            speedMps < WALK_MAX_MPS -> walkMs += ms
            else -> fastMs += ms
        }
    }

    fun add(f: Fix) {
        if (!f.accM.isFinite() || f.accM <= 0f || f.accM > MAX_ACC_M) return
        val prevT = lastT
        if (prevT != null && f.tMs <= prevT) return          // same fix from another provider, or out of order
        lastT = f.tMs
        if (dopplerOk(f)) {
            if (prevT != null && f.tMs - prevT <= MAX_GAP_MS) {
                credit(f.speedMps!!.toDouble(), min(f.tMs - prevT, MAX_DOPPLER_CREDIT_MS))
                viaDoppler++
            }
            anchor = f
            return
        }
        val a = anchor ?: run { anchor = f; return }
        val dtMs = f.tMs - a.tMs
        if (dtMs > MAX_GAP_MS) { anchor = f; return }
        val dtS = dtMs / 1000.0
        val d = distM(a, f)
        val noise = hypot(a.accM.toDouble(), f.accM.toDouble())
        when {
            d > 2 * noise -> {                                // moved for sure: at least walking
                credit(maxOf(d / dtS, STILL_MAX_MPS), dtMs); viaDisp++; anchor = f
            }
            STILL_MAX_MPS * dtS > 2 * noise -> {               // a walker would have shown by now
                stillMs += dtMs; viaDisp++; anchor = f
            }
            else -> Unit                                       // too soon to tell; keep the anchor
        }
    }

    /** The current window so far. */
    fun peek(): Summary {
        val s = (stillMs / 1000).toInt(); val w = (walkMs / 1000).toInt(); val q = (fastMs / 1000).toInt()
        val state = when {
            s + w + q < MIN_CLASSIFIED_S -> State.UNKNOWN
            s >= w && s >= q -> State.STILL
            w >= q -> State.WALKING
            else -> State.FAST
        }
        val via = when {
            viaDoppler == 0 && viaDisp == 0 -> "none"
            viaDoppler >= viaDisp -> "doppler"
            else -> "disp"
        }
        return Summary(state, round10(s), round10(w), round10(q), via)
    }

    /** Close the window: return its summary and start the next (the anchor carries over). */
    fun take(): Summary = peek().also { stillMs = 0; walkMs = 0; fastMs = 0; viaDoppler = 0; viaDisp = 0 }

    private fun round10(s: Int) = (s + 5) / 10 * 10
}

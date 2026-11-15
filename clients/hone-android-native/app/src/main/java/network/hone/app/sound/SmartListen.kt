package network.hone.app.sound

import kotlin.math.PI
import kotlin.math.asin
import kotlin.math.cos
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * Smart passive listening (Josh 14 Nov: "when there is NO bird noise, for example in a car, it should be smart and
 * turn off, and when birds are being heard again, turn on").
 *
 * On battery the listener no longer records 10 s every minute whatever happens. Instead:
 *  - **Back-off.** After a full burst (Perch on 2 x 5 s), the next one comes sooner or later depending on what
 *    it heard: a species new to this 15-min window -> again in 1 min; only species already heard -> at most
 *    every 5 min; nothing -> 2, 5, 10, then 20 min.
 *  - **Cheap probes.** While backed off, the phone records only 3 s and checks it with [BirdBand] (an FFT, no
 *    model): short tonal sounds in the bird band (1.5-10 kHz) that stand out of the background. Bird-like ->
 *    a full burst straight away and the back-off resets once Perch confirms a new species. Not bird-like -> back
 *    to waiting. The probe audio is RAM-only like every frame and is zeroed at once; only a yes/no comes out.
 *  - **Safety net.** A full burst at least every 30 min, so a bird the probe misses is still caught.
 *  - **Vehicles.** Moving faster than ~25 km/h (two fixes) -> no listening at all, on battery or power.
 * On power (charging, listening stations) the listener stays continuous, except in a vehicle.
 * Focus listening (Ornigo encounters) is never throttled.
 *
 * Numbers are OURS and can be tuned. Pure Kotlin so the rules are unit-tested.
 */
object SmartListen {
    const val PROBE_S = 3
    const val SAFETY_FULL_MS = 30 * 60_000L
    /** ~25 km/h: faster than walking or a relaxed bike ride. */
    const val VEHICLE_MPS = 7.0
    private const val VEHICLE_LOOK_MS = 5 * 60_000L
    private const val FIX_MAX_ACC_M = 150f

    /** Wait before the next listen, by how long nothing new was heard. */
    fun intervalMs(quiet: Int): Long = when {
        quiet <= 0 -> 60_000L
        quiet <= 2 -> 120_000L
        quiet <= 5 -> 300_000L
        quiet <= 9 -> 600_000L
        else -> 1_200_000L
    }

    /** After a full burst: a new species resets; only known species back off gently (<= 5 min); nothing backs off. */
    fun afterFull(quiet: Int, newSpecies: Boolean, anySpecies: Boolean): Int = when {
        newSpecies -> 0
        anySpecies -> minOf(quiet + 1, 3)
        else -> quiet + 1
    }

    /** A probe that heard nothing bird-like counts as a quiet round. */
    fun afterQuietProbe(quiet: Int) = quiet + 1

    /** Full burst (Perch) now, or only a 3 s probe? */
    fun fullNeeded(quiet: Int, sinceFullMs: Long): Boolean = quiet <= 0 || sinceFullMs >= SAFETY_FULL_MS

    // ── vehicle (from the location fixes the sightings engine already gets; no extra sensor) ──

    data class Fix(val atMs: Long, val lat: Double, val lon: Double, val accM: Float, val speedMps: Float? = null)

    private val fixes = ArrayDeque<Fix>()

    @Synchronized fun onFix(f: Fix) {
        if (f.accM > FIX_MAX_ACC_M) return
        fixes.addLast(f)
        while (fixes.size > 20 || (fixes.isNotEmpty() && f.atMs - fixes.first().atMs > 2 * VEHICLE_LOOK_MS)) fixes.removeFirst()
    }

    @Synchronized fun inVehicle(nowMs: Long): Boolean = vehicle(fixes.toList(), nowMs)

    /** Pure: moving faster than [VEHICLE_MPS] over the last 5 min (displacement well beyond the fixes' error),
     *  or a fresh fix that reports such a speed itself. */
    fun vehicle(all: List<Fix>, nowMs: Long): Boolean {
        val recent = all.filter { nowMs - it.atMs in 0..VEHICLE_LOOK_MS }.sortedBy { it.atMs }
        if (recent.isEmpty()) return false
        val last = recent.last()
        if (nowMs - last.atMs <= 120_000 && (last.speedMps ?: 0f) > VEHICLE_MPS) return true
        val first = recent.first()
        val dt = (last.atMs - first.atMs) / 1000.0
        if (dt < 45) return false
        val d = meters(first.lat, first.lon, last.lat, last.lon)
        return d > first.accM + last.accM + 200 && d / dt > VEHICLE_MPS
    }

    fun meters(lat1: Double, lon1: Double, lat2: Double, lon2: Double): Double {
        val r = 6_371_000.0
        val p1 = lat1 * PI / 180; val p2 = lat2 * PI / 180
        val dp = (lat2 - lat1) * PI / 180; val dl = (lon2 - lon1) * PI / 180
        val a = sin(dp / 2) * sin(dp / 2) + cos(p1) * cos(p2) * sin(dl / 2) * sin(dl / 2)
        return 2 * r * asin(sqrt(minOf(1.0, a)))
    }
}

/**
 * Cheap "could this be a bird?" check for a short probe: no model, one FFT per 32 ms block.
 * Bird-like = tonal energy in 1.5-10 kHz that either pops out of the background in short events (calls, song
 * phrases) or is strong and steady (a chorus). Engine and road noise sit below 1 kHz; wind and rain are not tonal.
 * Errs towards "yes": a false yes only costs one full burst.
 */
object BirdBand {
    private const val N = 1024
    const val LO_HZ = 1_500.0
    const val HI_HZ = 10_000.0
    private const val FLOOR_DB = -75.0          // OURS: below this the block is silence
    private const val EVENT_RATIO = 4.0         // OURS: block band energy vs the probe's median (+6 dB)
    private const val TONAL = 6.0               // OURS: band peak bin vs band mean
    private const val SHARE_EVENT = 0.25        // OURS: band share of the block's energy
    private const val MIN_EVENTS = 1
    private const val CHORUS_SHARE = 0.45
    private const val CHORUS_TONAL = 8.0

    private val hann = DoubleArray(N) { 0.5 - 0.5 * cos(2 * PI * it / (N - 1)) }

    fun active(x: FloatArray, sampleRate: Int = Perch.SAMPLE_RATE): Boolean = score(x, sampleRate).let {
        it.events >= MIN_EVENTS || it.chorus
    }

    data class Score(val events: Int, val chorus: Boolean, val blocks: Int)

    fun score(x: FloatArray, sampleRate: Int = Perch.SAMPLE_RATE): Score {
        val blocks = x.size / N
        if (blocks < 4) return Score(0, false, blocks)
        val hz = sampleRate.toDouble() / N
        val lo = (LO_HZ / hz).toInt().coerceAtLeast(1)
        val hi = (HI_HZ / hz).toInt().coerceAtMost(N / 2 - 1)
        val band = DoubleArray(blocks); val share = DoubleArray(blocks); val tonal = DoubleArray(blocks)
        val loud = BooleanArray(blocks)
        val re = DoubleArray(N); val im = DoubleArray(N)
        for (b in 0 until blocks) {
            var e = 0.0
            for (i in 0 until N) { val v = x[b * N + i].toDouble(); re[i] = v * hann[i]; im[i] = 0.0; e += v * v }
            loud[b] = 10 * kotlin.math.log10(e / N + 1e-12) > FLOOR_DB
            Playback.fft(re, im)
            var tot = 0.0; var be = 0.0; var peak = 0.0
            for (k in 1 until N / 2) {
                val p = re[k] * re[k] + im[k] * im[k]
                tot += p
                if (k in lo..hi) { be += p; if (p > peak) peak = p }
            }
            band[b] = be
            share[b] = if (tot > 0) be / tot else 0.0
            tonal[b] = if (be > 0) peak / (be / (hi - lo + 1)) else 0.0
        }
        val med = band.sorted()[blocks / 2]
        var events = 0
        var inEvent = false
        for (b in 0 until blocks) {
            val hit = loud[b] && band[b] > EVENT_RATIO * med && share[b] > SHARE_EVENT && tonal[b] > TONAL
            if (hit && !inEvent) events++
            inEvent = hit
        }
        val loudN = loud.count { it }
        val chorus = loudN >= blocks / 2 &&
            share.sorted()[blocks / 2] > CHORUS_SHARE && tonal.sorted()[blocks / 2] > CHORUS_TONAL
        return Score(events, chorus, blocks)
    }
}

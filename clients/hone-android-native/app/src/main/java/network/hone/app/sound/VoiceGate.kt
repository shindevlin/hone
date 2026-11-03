package network.hone.app.sound

import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.floor
import kotlin.math.log10
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sin

/**
 * Hone voice gate (sightings spec 2f, layer 5; "option 2: mute speech, keep the gaps").
 *
 * Finds human speech in one 5 s frame (32 kHz) so it is muted before Perch or the noise meter
 * sees the audio. No third-party VAD:
 *  1. Low-pass to ~1 kHz and decimate to 8 kHz. Voice pitch (70–500 Hz) and its first harmonics
 *     live there; most birdsong is above 1 kHz and is removed.
 *  2. 32 ms analysis frames every 10 ms. A frame is voiced when its normalized autocorrelation
 *     peak over lags for 70–500 Hz is ≥ [VOICED_R], its energy is above an absolute floor and either
 *     the frame's own noise floor + [FLOOR_DB] or [LOUD_DB] (so continuous voice with no pause is
 *     still caught), and most of its energy sits below 1 kHz ([LOW_SHARE_DB]).
 *  3. A 100 ms block with ≥ [MIN_VOICED] voiced frames is speech; it is widened by
 *     [MARGIN_BLOCKS] blocks each side (consonants, onsets) and muted.
 *  4. More than [DROP_FRACTION] of the blocks speech: the whole frame is dropped.
 *
 * Perch's own human classes stay as a second gate in [SoundSensing].
 * Reference implementation, golden values and the speech/bird test: `tools/voice-gate/`.
 * Keep this file and `tools/voice-gate/voice_gate.py` identical in behaviour (VoiceGateTest checks).
 */
object VoiceGate {
    const val SR = 32000
    const val BLOCK = 3200            // 100 ms
    const val DEC = 4                 // 32 kHz -> 8 kHz
    const val WIN = 256               // 32 ms at 8 kHz
    const val HOP = 80                // 10 ms at 8 kHz
    const val LAG_MIN = 16            // 500 Hz
    const val LAG_MAX = 114           // 70 Hz
    const val VOICED_R = 0.65
    const val ABS_FLOOR_DB = -70.0
    const val FLOOR_DB = 6.0
    const val LOW_SHARE_DB = -8.0
    const val LOUD_DB = -50.0
    const val MIN_VOICED = 3
    const val MARGIN_BLOCKS = 3
    const val DROP_FRACTION = 0.5
    private const val NTAPS = 63

    class Result(
        /** One entry per 100 ms block; true = speech, muted. */
        val mask: BooleanArray,
        /** Mostly speech: discard the whole frame. */
        val drop: Boolean,
        val voiced: BooleanArray,
        val peak: DoubleArray,
        val energyDb: DoubleArray,
    ) {
        val mutedBlocks: Int get() = mask.count { it }

        /** Zero every speech block in place. */
        fun mute(frame: FloatArray) {
            for (b in mask.indices) if (mask[b]) frame.fill(0f, b * BLOCK, min((b + 1) * BLOCK, frame.size))
        }
    }

    // Windowed-sinc low-pass, cutoff 1 kHz at 32 kHz, Hamming window, unity DC gain.
    private val taps = DoubleArray(NTAPS).also { h ->
        val fc = 1000.0 / SR
        for (i in 0 until NTAPS) {
            val m = i - (NTAPS - 1) / 2.0
            val a = 2 * fc * m
            val sinc = if (a == 0.0) 1.0 else sin(PI * a) / (PI * a)
            h[i] = 2 * fc * sinc * (0.54 - 0.46 * cos(2 * PI * i / (NTAPS - 1)))
        }
        val s = h.sum()
        for (i in 0 until NTAPS) h[i] /= s
    }
    private val hann = DoubleArray(WIN) { 0.5 - 0.5 * cos(2 * PI * it / (WIN - 1)) }

    fun analyze(x: FloatArray): Result {
        // 1. low-pass ("same" convolution) and keep every 4th sample
        val half = (NTAPS - 1) / 2
        val ny = (x.size + DEC - 1) / DEC
        val y = DoubleArray(ny)
        for (q in 0 until ny) {
            val i = q * DEC
            var acc = 0.0
            for (j in 0 until NTAPS) {
                val k = i + half - j
                if (k >= 0 && k < x.size) acc += taps[j] * x[k]
            }
            y[q] = acc
        }
        // 2. analysis frames
        val n = if (ny >= WIN) 1 + (ny - WIN) / HOP else 0
        val peak = DoubleArray(n)
        val edb = DoubleArray(n)
        val share = DoubleArray(n)
        val f = DoubleArray(WIN)
        for (i in 0 until n) {
            val off = i * HOP
            var mean = 0.0
            for (k in 0 until WIN) mean += y[off + k]
            mean /= WIN
            var e = 0.0
            for (k in 0 until WIN) {
                val v = (y[off + k] - mean) * hann[k]
                f[k] = v; e += v * v
            }
            edb[i] = 10 * log10(e / WIN + 1e-20)
            var best = Double.NEGATIVE_INFINITY
            for (l in LAG_MIN..LAG_MAX) {
                var ac = 0.0
                for (k in 0 until WIN - l) ac += f[k] * f[k + l]
                val r = ac / (e + 1e-20) * (WIN.toDouble() / max(WIN - l, 1))
                if (r > best) best = r
            }
            peak[i] = best
            val start = i * HOP * DEC
            var fe = 0.0
            for (k in 0 until WIN * DEC) {
                val v = x[min(start + k, x.size - 1)].toDouble()
                fe += v * v
            }
            share[i] = edb[i] - 10 * log10(fe / (WIN * DEC) + 1e-20)
        }
        // noise floor: 10th percentile of frame energies (linear interpolation, as numpy)
        val noiseFloor = if (n == 0) 0.0 else {
            val s = edb.sortedArray()
            val pos = 0.1 * (n - 1)
            val lo = floor(pos).toInt()
            val hi = min(lo + 1, n - 1)
            s[lo] + (pos - lo) * (s[hi] - s[lo])
        }
        val voiced = BooleanArray(n) {
            peak[it] >= VOICED_R && edb[it] >= ABS_FLOOR_DB &&
                (edb[it] >= noiseFloor + FLOOR_DB || edb[it] >= LOUD_DB) &&
                share[it] >= LOW_SHARE_DB
        }
        // 3. blocks, margin
        val nBlocks = x.size / BLOCK
        val hopsPerBlock = BLOCK / DEC / HOP
        val core = BooleanArray(nBlocks) { b ->
            var c = 0
            for (h in b * hopsPerBlock until min((b + 1) * hopsPerBlock, n)) if (voiced[h]) c++
            c >= MIN_VOICED
        }
        val mask = BooleanArray(nBlocks)
        for (b in 0 until nBlocks) if (core[b]) {
            for (k in max(0, b - MARGIN_BLOCKS)..min(nBlocks - 1, b + MARGIN_BLOCKS)) mask[k] = true
        }
        // 4. mostly speech: drop
        val drop = nBlocks > 0 && mask.count { it }.toDouble() / nBlocks > DROP_FRACTION
        return Result(mask, drop, voiced, peak, edb)
    }
}

package network.hone.app.sound

import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.sin

/**
 * Speaker-signature score of one frame, 0..1 (Birderlands proposal hard-birds §1.2, applied 13 Nov).
 *
 * A call played through a phone, TV or radio speaker, or from compressed audio, has almost no energy below
 * ~150 Hz and a hard cut above ~12 kHz. A live microphone outdoors has some of both (wind, rumble, hiss).
 * The frame's spectrum (averaged 4096-point FFTs, Hann window) gives the share of energy in those two bands.
 * Nothing is kept: only the score leaves this function.
 *
 * SHADOW MODE: uncalibrated. Sent with sightings as `playback_q8` so Hone can calibrate it on real reports;
 * the indexer does not hold anything on it until it is calibrated. Numbers marked OURS can be tuned.
 */
object Playback {
    const val LOW_HZ = 150.0          // OURS
    const val HIGH_HZ = 12_000.0      // OURS
    const val LOW_FULL = 0.02         // OURS: a live mic usually has at least this share of energy below LOW_HZ
    const val HIGH_FULL = 0.003       // OURS: ... and at least this share above HIGH_HZ
    private const val N = 4096
    private const val MAX_WINDOWS = 16  // 16 x 128 ms is plenty for a 5 s frame and keeps it cheap

    private val hann = DoubleArray(N) { 0.5 - 0.5 * cos(2 * PI * it / (N - 1)) }

    fun score(x: FloatArray, sampleRate: Int = Perch.SAMPLE_RATE): Float {
        if (x.size < N) return 0f
        val bins = N / 2
        val power = DoubleArray(bins)
        val re = DoubleArray(N); val im = DoubleArray(N)
        val windows = minOf(MAX_WINDOWS, x.size / N)
        val step = (x.size - N) / maxOf(1, windows - 1)
        for (w in 0 until windows) {
            val off = w * step
            for (i in 0 until N) { re[i] = x[off + i] * hann[i]; im[i] = 0.0 }
            fft(re, im)
            for (k in 1 until bins) power[k] += re[k] * re[k] + im[k] * im[k]
        }
        val hz = sampleRate.toDouble() / N
        var total = 0.0; var low = 0.0; var high = 0.0
        for (k in 1 until bins) {
            total += power[k]
            val f = k * hz
            if (f < LOW_HZ) low += power[k]
            if (f > HIGH_HZ) high += power[k]
        }
        if (total < 1e-9) return 0f
        val sLow = (1 - (low / total) / LOW_FULL).coerceIn(0.0, 1.0)
        val sHigh = (1 - (high / total) / HIGH_FULL).coerceIn(0.0, 1.0)
        return (0.5 * sLow + 0.5 * sHigh).toFloat()
    }

    /** In-place iterative radix-2 FFT (size must be a power of two). */
    internal fun fft(re: DoubleArray, im: DoubleArray) {
        val n = re.size
        var j = 0
        for (i in 1 until n) {
            var bit = n shr 1
            while (j and bit != 0) { j = j xor bit; bit = bit shr 1 }
            j = j xor bit
            if (i < j) { var t = re[i]; re[i] = re[j]; re[j] = t; t = im[i]; im[i] = im[j]; im[j] = t }
        }
        var len = 2
        while (len <= n) {
            val ang = -2 * PI / len
            val wr = cos(ang); val wi = sin(ang)
            var i = 0
            while (i < n) {
                var cr = 1.0; var ci = 0.0
                for (k in 0 until len / 2) {
                    val ar = re[i + k + len / 2] * cr - im[i + k + len / 2] * ci
                    val ai = re[i + k + len / 2] * ci + im[i + k + len / 2] * cr
                    re[i + k + len / 2] = re[i + k] - ar; im[i + k + len / 2] = im[i + k] - ai
                    re[i + k] += ar; im[i + k] += ai
                    val nr = cr * wr - ci * wi; ci = cr * wi + ci * wr; cr = nr
                }
                i += len
            }
            len = len shl 1
        }
    }
}

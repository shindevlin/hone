package network.hone.app.station

import kotlin.math.log10

/** ITU-T G.711 decoders (A-law, µ-law) to 16-bit linear. */
object G711 {
    fun alaw(b: Int): Int {
        val a = (b and 0xFF) xor 0x55
        var t = (a and 0x0F) shl 4
        val seg = (a and 0x70) shr 4
        t = when (seg) { 0 -> t + 8; 1 -> t + 0x108; else -> (t + 0x108) shl (seg - 1) }
        return if (a and 0x80 != 0) t else -t
    }

    fun ulaw(b: Int): Int {
        val u = (b and 0xFF) xor 0xFF
        var t = ((u and 0x0F) shl 3) + 0x84
        t = t shl ((u and 0x70) shr 4)
        return if (u and 0x80 != 0) 0x84 - t else t - 0x84
    }

    /** RTP payload -> mono floats in [-1, 1]. */
    fun decode(encoding: String, payload: ByteArray, channels: Int = 1): FloatArray = when (encoding) {
        "PCMA" -> FloatArray(payload.size) { alaw(payload[it].toInt()) / 32768f }
        "PCMU" -> FloatArray(payload.size) { ulaw(payload[it].toInt()) / 32768f }
        "L16" -> {
            val ch = channels.coerceAtLeast(1)
            val n = payload.size / 2 / ch
            FloatArray(n) { i ->
                var s = 0f
                for (c in 0 until ch) {
                    val k = (i * ch + c) * 2
                    s += (((payload[k].toInt() shl 8) or (payload[k + 1].toInt() and 0xFF)).toShort()) / 32768f
                }
                s / ch
            }
        }
        else -> FloatArray(0)
    }
}

/** Streaming linear resampler (continuous across calls). */
class LinearResampler(private val inRate: Int, private val outRate: Int) {
    private val step = inRate.toDouble() / outRate
    private var t = 0.0          // position in [prev, x0, x1, ...]
    private var prev = 0f

    fun process(x: FloatArray): FloatArray {
        if (inRate == outRate || x.isEmpty()) return x
        val out = FloatArray(((x.size - t) / step).toInt().coerceAtLeast(0) + 2)
        var n = 0
        while (t < x.size && n < out.size) {
            val i = t.toInt(); val f = (t - i).toFloat()
            val a = if (i == 0) prev else x[i - 1]
            out[n++] = a + (x[i] - a) * f
            t += step
        }
        t -= x.size
        prev = x.last()
        return out.copyOf(n)
    }
}

object StationQuality {
    fun tier(rate: Int) = when { rate >= 32_000 -> "A"; rate >= 16_000 -> "B"; else -> "C" }

    /** 10th-percentile level of 100 ms blocks, dBFS (the quiet background). */
    fun noiseFloorDb(x: FloatArray, rate: Int): Double? {
        val block = (rate / 10).coerceAtLeast(1)
        val levels = (0 until x.size / block).map { b ->
            var e = 0.0
            for (k in b * block until (b + 1) * block) e += x[k].toDouble() * x[k]
            10 * log10(e / block + 1e-12)
        }.sorted()
        if (levels.isEmpty()) return null
        return Math.round(levels[(levels.size * 0.1).toInt()] * 10) / 10.0
    }
}

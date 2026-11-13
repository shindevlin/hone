package network.hone.app.sound

import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.PI
import kotlin.math.sin
import kotlin.random.Random

class PlaybackTest {
    private val sr = 32_000

    private fun white(seconds: Int, seed: Int) = Random(seed).let { r -> FloatArray(seconds * sr) { (r.nextFloat() * 2 - 1) * 0.3f } }

    /** Tones only between 400 Hz and 8 kHz: what a small speaker / compressed track leaves. */
    private fun bandLimited(seconds: Int): FloatArray {
        val f = listOf(400.0, 900.0, 1800.0, 3200.0, 5000.0, 7500.0)
        return FloatArray(seconds * sr) { i -> (f.sumOf { sin(2 * PI * it * i / sr) } / f.size * 0.3).toFloat() }
    }

    @Test fun fullBandLiveSoundScoresLow() {
        val s = Playback.score(white(5, 1))
        assertTrue("white noise scored $s", s < 0.4f)
    }

    @Test fun bandLimitedSoundScoresHigh() {
        val s = Playback.score(bandLimited(5))
        assertTrue("band-limited scored $s", s > 0.8f)
    }

    @Test fun birdOverRumbleAndHissScoresLow() {
        val w = white(5, 2)
        val b = bandLimited(5)
        val rumble = FloatArray(5 * sr) { i -> (sin(2 * PI * 60 * i / sr) * 0.2).toFloat() }
        val s = Playback.score(FloatArray(b.size) { b[it] + rumble[it] + w[it] * 0.3f })
        assertTrue("live mix scored $s", s < 0.5f)
    }

    @Test fun silenceScoresZero() {
        assertTrue(Playback.score(FloatArray(sr)) == 0f)
    }
}

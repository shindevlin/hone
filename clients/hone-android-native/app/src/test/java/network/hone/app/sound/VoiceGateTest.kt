package network.hone.app.sound

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.PI
import kotlin.math.sin

/**
 * The Kotlin gate must behave exactly like the Python reference (tools/voice-gate/voice_gate.py).
 * Expected values come from tools/voice-gate/gen_vectors.py on the same synthetic frame.
 */
class VoiceGateTest {

    private fun bird(t: Double) = if ((t * 4) % 1.0 < 0.5) 0.05 * sin(2 * PI * 3000 * t) else 0.0

    private fun stack(t: Double, f0: Double): Double {
        var v = 0.0
        for (k in 1..8) v += (0.1 / k) * sin(2 * PI * k * f0 * t)
        return v
    }

    /** 3 kHz "bird" on for the first half of every 250 ms, plus a 140 Hz harmonic "voice" in 2.0–3.0 s. */
    private fun synth(): FloatArray = FloatArray(5 * VoiceGate.SR) { i ->
        val t = i.toDouble() / VoiceGate.SR
        val voice = if (t >= 2.0 && t < 3.0) stack(t, 140.0) else 0.0
        (bird(t) + voice).toFloat()
    }

    @Test fun matchesPythonReference() {
        val r = VoiceGate.analyze(synth())
        val expected = BooleanArray(50) { it in 17..32 }
        assertArrayEquals(expected, r.mask)
        assertFalse(r.drop)
        assertEquals(99, r.voiced.count { it })
        val hops = intArrayOf(0, 100, 210, 250, 290, 400, 496)
        val peak = doubleArrayOf(1.040886778164853, 1.0408867781649183, 0.9228970135198005, 0.922868344385425,
            0.922864085565105, 1.0408867781629487, 0.0)
        val edb = doubleArrayOf(-89.79277637580643, -89.79277637580506, -25.712545642441263, -25.711693068017382,
            -25.71218559823333, -89.79277637542879, -200.0)
        for ((k, h) in hops.withIndex()) {
            assertEquals("peak at hop $h", peak[k], r.peak[h], 1e-6)
            assertEquals("energy at hop $h", edb[k], r.energyDb[h], 1e-6)
        }
    }

    @Test fun birdOnlyIsKept() {
        val f = FloatArray(5 * VoiceGate.SR) { bird(it.toDouble() / VoiceGate.SR).toFloat() }
        val r = VoiceGate.analyze(f)
        assertEquals(0, r.mutedBlocks)
        assertFalse(r.drop)
    }

    @Test fun continuousVoiceDropsTheFrame() {
        val f = FloatArray(5 * VoiceGate.SR) { stack(it.toDouble() / VoiceGate.SR, 130.0).toFloat() }
        assertTrue(VoiceGate.analyze(f).drop)
    }

    @Test fun muteZeroesOnlyMaskedBlocks() {
        val f = synth()
        val r = VoiceGate.analyze(f)
        r.mute(f)
        for (b in 0 until 50) {
            val zero = (b * VoiceGate.BLOCK until (b + 1) * VoiceGate.BLOCK).all { f[it] == 0f }
            if (r.mask[b]) assertTrue("block $b muted", zero)
        }
        assertFalse("block 5 keeps the bird", (5 * VoiceGate.BLOCK until 6 * VoiceGate.BLOCK).all { f[it] == 0f })
    }
}

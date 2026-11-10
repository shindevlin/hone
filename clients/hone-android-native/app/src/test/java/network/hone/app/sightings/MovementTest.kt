package network.hone.app.sightings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.cos
import kotlin.random.Random

class MovementTest {
    private val lat0 = 13.6929
    private val lon0 = -89.2182
    private fun north(m: Double) = m / 111_195.0
    private fun east(m: Double) = m / (111_195.0 * cos(Math.toRadians(lat0)))

    private fun fix(t: Long, n: Double, e: Double, acc: Float, v: Float? = null, va: Float? = null) =
        Movement.Fix(t, lat0 + north(n), lon0 + east(e), acc, v, va)

    @Test fun standingStillWithGpsJitterIsStill() {
        val m = Movement(); val r = Random(1)
        for (i in 0..45) m.add(fix(i * 20_000L, r.nextDouble(-4.0, 4.0), r.nextDouble(-4.0, 4.0), 6f))
        val s = m.take()
        assertEquals(Movement.State.STILL, s.state)
        assertTrue("most of 15 min classified: $s", s.stillS >= 600)
        assertEquals(0, s.walkS); assertEquals(0, s.fastS)
    }

    @Test fun slowBirdingWalkByDisplacementIsWalking() {
        val m = Movement()
        for (i in 0..15) m.add(fix(i * 60_000L, i * 60 * 0.8, 0.0, 10f))   // 0.8 m/s, no Doppler
        val s = m.take()
        assertEquals(Movement.State.WALKING, s.state)
        assertEquals("disp", s.via)
    }

    @Test fun dopplerSpeedInACarIsFast() {
        val m = Movement()
        for (i in 0..30) m.add(fix(i * 30_000L, i * 30 * 12.0, 0.0, 5f, 12f, 0.4f))
        val s = m.take()
        assertEquals(Movement.State.FAST, s.state)
        assertEquals("doppler", s.via)
    }

    @Test fun dopplerNearZeroIsStill() {
        val m = Movement()
        for (i in 0..30) m.add(fix(i * 30_000L, 0.0, 0.0, 4f, 0.1f, 0.3f))
        assertEquals(Movement.State.STILL, m.take().state)
    }

    @Test fun poorFixesNeverInventMovementOrStillness() {
        val m = Movement(); val r = Random(2)
        for (i in 0..45) m.add(fix(i * 20_000L, r.nextDouble(-60.0, 60.0), r.nextDouble(-60.0, 60.0), 90f))
        val s = m.take()
        assertEquals(Movement.State.UNKNOWN, s.state)
        assertEquals(0, s.stillS + s.walkS + s.fastS)
    }

    @Test fun duplicateFixesFromTwoProvidersCountOnce() {
        val a = Movement(); val b = Movement()
        for (i in 0..15) {
            val f = fix(i * 60_000L, i * 60.0, 0.0, 8f)
            a.add(f); b.add(f); b.add(f)
        }
        assertEquals(a.take(), b.take())
    }

    @Test fun longGapsAreNotCredited() {
        val m = Movement()
        m.add(fix(0, 0.0, 0.0, 5f, 0f, 0.2f))
        m.add(fix(600_000L, 0.0, 0.0, 5f, 0f, 0.2f))      // 10 min later: nothing credited
        assertEquals(Movement.State.UNKNOWN, m.take().state)
    }

    @Test fun badSpeedAccuracyFallsBackToDisplacement() {
        val m = Movement()
        for (i in 0..15) m.add(fix(i * 60_000L, 0.0, 0.0, 6f, 9f, 5f))   // noisy speed claims 9 m/s, position says still
        val s = m.take()
        assertEquals(Movement.State.STILL, s.state)
        assertEquals("disp", s.via)
    }

    @Test fun windowResetKeepsAnchor() {
        val m = Movement()
        for (i in 0..15) m.add(fix(i * 60_000L, i * 60.0, 0.0, 8f))
        m.take()
        assertEquals(Movement.State.UNKNOWN, m.peek().state)
        for (i in 16..31) m.add(fix(i * 60_000L, i * 60.0, 0.0, 8f))
        assertEquals(Movement.State.WALKING, m.take().state)
    }
}

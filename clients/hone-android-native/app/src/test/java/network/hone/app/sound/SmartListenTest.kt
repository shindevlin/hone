package network.hone.app.sound

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.PI
import kotlin.math.sin
import kotlin.random.Random

class SmartListenTest {
    private val sr = 32_000

    @Test fun backsOffWhenNothingIsHeardAndResetsOnANewSpecies() {
        var q = 0
        val waits = ArrayList<Long>()
        repeat(12) { q = SmartListen.afterFull(q, newSpecies = false, anySpecies = false); waits.add(SmartListen.intervalMs(q)) }
        assertEquals(120_000L, waits[0])
        assertEquals(1_200_000L, waits.last())
        assertEquals(0, SmartListen.afterFull(q, newSpecies = true, anySpecies = true))
        assertEquals(60_000L, SmartListen.intervalMs(0))
    }

    @Test fun knownSpeciesOnlyBackOffGently() {
        var q = 0
        repeat(10) { q = SmartListen.afterFull(q, newSpecies = false, anySpecies = true) }
        assertTrue(SmartListen.intervalMs(q) <= 300_000L)
    }

    @Test fun probesUntilTheSafetyNet() {
        assertTrue(SmartListen.fullNeeded(0, 0))
        assertFalse(SmartListen.fullNeeded(4, 10 * 60_000L))
        assertTrue(SmartListen.fullNeeded(4, 30 * 60_000L))
    }

    @Test fun vehicleFromFixes() {
        val t0 = 1_000_000L
        // 60 km/h for 2 min along a meridian (1 deg lat ~ 111 km)
        val drive = (0..4).map { SmartListen.Fix(t0 + it * 30_000L, 13.70 + it * 500.0 / 111_000, -89.2, 40f) }
        assertTrue(SmartListen.vehicle(drive, t0 + 120_000))
        // walking 5 km/h with 50 m fix noise
        val walk = (0..4).map { SmartListen.Fix(t0 + it * 30_000L, 13.70 + it * 42.0 / 111_000 + (if (it % 2 == 0) 0.0004 else 0.0), -89.2, 50f) }
        assertFalse(SmartListen.vehicle(walk, t0 + 120_000))
        // a fresh fix that reports its own speed
        assertTrue(SmartListen.vehicle(listOf(SmartListen.Fix(t0, 13.7, -89.2, 20f, speedMps = 15f)), t0 + 10_000))
        // old fixes don't count
        assertFalse(SmartListen.vehicle(drive, t0 + 30 * 60_000L))
    }

    @Test fun aProbeThatKeepsFiringOnNonBirdsIsIgnoredForAWhile() {
        assertTrue(SmartListen.probeTrusted(1, 0))
        assertFalse(SmartListen.probeTrusted(2, 60_000L))
        assertTrue(SmartListen.probeTrusted(2, 3 * 3600_000L))
    }

    private fun t(i: Int) = i.toDouble() / sr

    private fun chirps(amp: Double, noise: Double, r: Random): FloatArray {
        val x = FloatArray(3 * sr) { (r.nextDouble(-1.0, 1.0) * noise * 1.7).toFloat() }
        for (k in 0 until 5) {
            val s = ((0.2 + k * 0.55) * sr).toInt(); val len = (0.08 * sr).toInt(); var ph = 0.0
            for (i in 0 until len) { val f = 3000 + 2000.0 * i / len; ph += 2 * PI * f / sr; x[s + i] += (amp * sin(ph)).toFloat() }
        }
        return x
    }

    @Test fun birdBandHearsCallsButNotCarsOrSilence() {
        val r = Random(1)
        assertTrue(BirdBand.active(chirps(0.05, 0.003, r)))
        assertTrue(BirdBand.active(chirps(0.01, 0.003, r)))
        val car = FloatArray(3 * sr) { i -> (0.08 * sin(2 * PI * 80 * t(i)) + 0.04 * sin(2 * PI * 160 * t(i)) + r.nextDouble(-0.02, 0.02)).toFloat() }
        // road rumble: low-passed noise
        var lp = 0.0
        for (i in car.indices) { lp = 0.98 * lp + 0.02 * r.nextDouble(-1.0, 1.0); car[i] += (lp * 2).toFloat() }
        assertFalse(BirdBand.active(car))
        assertFalse(BirdBand.active(FloatArray(3 * sr)))
        assertFalse(BirdBand.active(FloatArray(3 * sr) { (r.nextDouble(-0.02, 0.02)).toFloat() }))
    }

    @Test fun birdBandHearsASteadyChorus() {
        val r = Random(2)
        val x = FloatArray(3 * sr) { i ->
            (listOf(2500.0, 3400.0, 4300.0, 5200.0).withIndex().sumOf { (k, f) -> 0.01 * sin(2 * PI * f * t(i) + 3 * sin(2 * PI * (5 + k) * t(i))) } +
                r.nextDouble(-0.003, 0.003)).toFloat()
        }
        assertTrue(BirdBand.active(x))
    }
}

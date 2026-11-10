package network.hone.app.raw

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.random.Random

class RawFmtTest {
    @Test fun anyMomentRebuildsExactlyFromKeyframePlusDeltas() {
        val rnd = Random(7)
        val enc = ChangeEncoder()
        val recs = ArrayList<Map<String, Any?>>()
        val snaps = ArrayList<Pair<Long, Map<String, RawFmt.Entry>>>()
        var t = 1_794_000_000_000L
        enc.keyframe(t, "boot"); recs += enc.take(); snaps += t to LinkedHashMap(enc.reported)
        val aps = (0 until 40).associate { "wifi:%02x".format(it) to rnd.nextDouble(-90.0, -40.0) }
        var temp = 24.0
        for (step in 0 until 3000) {
            t += listOf(1_000L, 5_000L, 30_000L, 60_000L).random(rnd)
            val vis = aps.filter { rnd.nextDouble() < 0.7 }.mapValues { (_, v) -> (v + rnd.nextDouble(-8.0, 8.0)) to mapOf<String, Any?>("f" to 2437) }
            enc.observeSet(t, "wifi", vis)
            temp += rnd.nextDouble(-0.15, 0.15); enc.observe(t, "env.air_temp_c", Math.round(temp * 100) / 100.0)
            enc.observe(t, "env.light_lux", maxOf(0.0, 300 + rnd.nextDouble(-120.0, 120.0)))
            enc.observe(t, "dev.temp_c", 30 + rnd.nextDouble(-3.0, 3.0))
            if (step % 15 == 0) enc.endWindow(t, listOf("wifi"))
            if (step % 400 == 0) enc.gap(t - 30_000, t, "ble", "permission_denied")
            if (step == 1500) enc.keyframe(t, "day")
            recs += enc.take(); snaps += t to LinkedHashMap(enc.reported)
        }
        for ((ts, snap) in snaps.filterIndexed { i, _ -> i % 37 == 0 } + snaps.last())
            assertEquals("state at $ts", snap, RawFmt.rebuild(recs, ts))
        assertTrue(recs.all { it["s"] in setOf(RawFmt.SCHEMA_KF, RawFmt.SCHEMA_EV) })
        assertTrue(recs.filter { it["s"] == RawFmt.SCHEMA_EV }.all { it["e"] in RawFmt.EVENTS })
    }

    @Test fun deadbandsAndDisappearance() {
        val enc = ChangeEncoder(); enc.keyframe(0, "boot"); enc.take()
        enc.observe(1_000, "env.air_temp_c", 20.0); enc.observe(2_000, "env.air_temp_c", 20.1); enc.observe(3_000, "env.air_temp_c", 20.2)
        assertEquals(listOf("appeared", "changed"), enc.take().map { it["e"] })
        enc.observe(4_000, "env.pressure_pa", 101300.0); enc.observe(5_000, "env.pressure_pa", 101309.0); enc.observe(6_000, "env.pressure_pa", 101311.0)
        assertEquals(listOf("appeared", "changed"), enc.take().map { it["e"] })      // 0.1 hPa = 10 Pa
        enc.observe(7_000, "env.light_lux", 100.0); enc.observe(8_000, "env.light_lux", 109.0); enc.observe(9_000, "env.light_lux", 111.0)
        assertEquals(listOf("appeared", "changed"), enc.take().map { it["e"] })      // 10 %
        val a = mapOf<String, Any?>()
        enc.observeSet(10_000, "wifi", mapOf("wifi:a" to (-50.0 to a))); enc.observeSet(11_000, "wifi", mapOf("wifi:a" to (-55.0 to a)))
        enc.observeSet(12_000, "wifi", mapOf("wifi:a" to (-57.0 to a)))
        assertEquals(listOf("appeared", "changed"), enc.take().map { it["e"] })      // > 6 dB from the reported -50
        enc.observeSet(13_000, "wifi", emptyMap()); assertTrue(enc.take().isEmpty())  // one miss is not gone
        enc.observeSet(14_000, "wifi", emptyMap()); assertEquals(listOf("disappeared"), enc.take().map { it["e"] })
    }

    @Test fun markersForcedSamplesAndGaps() {
        val enc = ChangeEncoder(); enc.keyframe(0, "boot"); enc.take()
        enc.observe(10_000, "env.rh_pct", 61.0)
        enc.observeSet(10_000, "wifi", mapOf("wifi:a" to (-60.0 to emptyMap())))
        enc.take()
        enc.endWindow(900_000, listOf("wifi", "ble"))
        enc.gap(0, 900_000, "audio", "speech_deleted")
        val r = enc.take()
        assertTrue(r.any { it["k"] == "wifi:*" && it["e"] == "unchanged" && it["n"] == 1 })
        assertTrue(r.any { it["k"] == "env.rh_pct" && it["forced"] == true && it["v"] == 61.0 })
        assertTrue(r.any { it["e"] == "gap" && it["reason"] == "speech_deleted" && it["k"] == "audio:*" })
        assertFalse(r.any { it["k"] == "ble:*" })     // nothing known in ble: no marker
    }

    @Test fun jsonIsSortedCompactAndTokensHideIdentifiers() {
        assertEquals("""{"a":1,"b":-50,"c":20.15,"d":"x\"y","e":null}""",
            RawFmt.json(mapOf("e" to null, "b" to -50.0, "a" to 1, "d" to "x\"y", "c" to 20.15)))
        val tok = RawFmt.token("876d74365ffffff", "2026-W46", "aa:bb:cc:dd:ee:ff")
        assertEquals(12, tok.length); assertFalse(tok.contains(":"))
        assertEquals("2026-11-10T16:00:00.000Z", RawFmt.rfc3339(1_794_326_400_000L))
        assertEquals(1_794_326_400_123L, RawFmt.parse(RawFmt.rfc3339(1_794_326_400_123L)))
    }
}

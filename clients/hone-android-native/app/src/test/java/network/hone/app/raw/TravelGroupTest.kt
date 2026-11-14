package network.hone.app.raw

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TravelGroupTest {
    private fun fix(t: Long, lat: Double, lon: Double, spd: Float = 0f) = TravelGroup.Fix(t, lat, lon, 10f, spd)

    @Test fun adaptiveThinning() {
        val a = fix(0, 13.70, -89.24)
        assertTrue(TravelGroup.keep(null, a))
        assertFalse(TravelGroup.keep(a, fix(30_000, 13.70, -89.24)))                 // still, 30 s
        assertTrue(TravelGroup.keep(a, fix(600_000, 13.70, -89.24)))                 // still, 10 min
        assertFalse(TravelGroup.keep(a, fix(30_000, 13.7001, -89.24, 1.4f)))         // walking, 30 s, 11 m
        assertTrue(TravelGroup.keep(a, fix(60_000, 13.7001, -89.24, 1.4f)))          // walking, 1 min
        assertTrue(TravelGroup.keep(a, fix(20_000, 13.7003, -89.24)))                // 33 m move
    }

    @Test fun interpolatesBetweenFixes() {
        val tr = listOf(fix(0, 13.0, -89.0), fix(600_000, 13.01, -89.01))
        val p = TravelGroup.positionAt(tr, 300_000)!!
        assertEquals(13.005, p.first, 1e-9); assertEquals(-89.005, p.second, 1e-9)
        assertNotNull(TravelGroup.positionAt(tr, 600_000 + 200_000))                   // within 5 min of the end
        assertNull(TravelGroup.positionAt(tr, 600_000 + 400_000))
        val gap = listOf(fix(0, 13.0, -89.0), fix(3_600_000, 13.1, -89.1))
        assertNull(TravelGroup.positionAt(gap, 1_800_000))                            // 30 min from both: unknown
        assertEquals(13.0, TravelGroup.positionAt(gap, 200_000)!!.first, 1e-9)        // nearest within 5 min
    }

    @Test fun colocationMatchesThePythonScorer() {
        val a = TravelGroup.Env((0 until 20).map { "wifi:$it" }.toSet(), 30, listOf(100_000, 400_000, 900_000))
        val b = TravelGroup.Env((5 until 25).map { "wifi:$it" }.toSet(), 26, listOf(110_000, 905_000))
        val far = TravelGroup.Env((0 until 20).map { "wifi:x$it" }.toSet(), 3, listOf(5_000_000))
        val m = TravelGroup.colocate(a, b)
        assertTrue(m.match); assertEquals(64, m.proof!!.length)
        assertEquals(0.6, m.parts["wifi"]!!, 1e-4)                                    // 15 / 25
        assertFalse(TravelGroup.colocate(a, far).match)
        assertFalse(TravelGroup.colocate(TravelGroup.Env(), TravelGroup.Env()).match)
    }

    @Test fun tierBNeedsTrackMatchAndHomeAnchor() {
        val tr = listOf(fix(0, 13.0, -89.0), fix(600_000, 13.01, -89.01))
        val env = TravelGroup.Env((0 until 10).map { "wifi:$it" }.toSet(), 10)
        val home = TravelGroup.Env((100 until 110).map { "wifi:$it" }.toSet(), 5)
        val b = TravelGroup.place(tr, 300_000, env, env, home, home)
        assertEquals("B", b.tier); assertEquals("travel_group", b.src); assertNotNull(b.proof)
        assertEquals("C", TravelGroup.place(tr, 300_000, env, env, env, home).tier)        // no home anchor
        assertEquals("C", TravelGroup.place(tr, 300_000, env, home, home, home).tier)      // radio mismatch
        assertEquals("C", TravelGroup.place(emptyList(), 300_000, env, env, home, home).tier) // no track
    }

    @Test fun obsTypes() {
        assertEquals("pressure", RawFmt.obsTypeOf("env.pressure_pa"))
        assertEquals("cell_coverage", RawFmt.obsTypeOf("cell:abc"))
        assertEquals("wifi_env", RawFmt.obsTypeOf("wifi:*"))
        assertEquals("presence", RawFmt.obsTypeOf("flipper:ab12"))
        assertTrue(RawFmt.forced("cell:abc"))
        assertTrue(RawFmt.OBS_TYPES.containsAll(listOf("pressure", "cell_coverage", "foot_traffic", "outage_signal", "aircraft")))
    }

    @Test fun bestSourcePerObsType() {
        val phone = Capabilities.Dev("p1", "phone", setOf("wifi", "ble", "gps", "cell", "baro", "mic"))
        val board = Capabilities.Dev("f1", "flipper", Capabilities.flipper(listOf("subghz", "nfc", "rfid", "ir", "wifi")))
        val bare = Capabilities.Dev("f2", "flipper", Capabilities.flipper(listOf("subghz", "nfc", "ir")))
        assertTrue("wifi_board" in board.caps); assertFalse("wifi_board" in bare.caps)
        assertEquals("f1", Capabilities.best("wifi_env", listOf(phone, board, bare)).device)
        assertEquals("p1", Capabilities.best("wifi_env", listOf(phone, bare)).device)              // no board: phone scans
        assertEquals("high", Capabilities.best("foot_traffic", listOf(phone, board)).precision)
        assertEquals("low", Capabilities.best("foot_traffic", listOf(phone, bare)).precision)      // BLE estimate
        assertEquals("p1", Capabilities.best("location", listOf(bare, phone)).device)
        assertEquals("f2", Capabilities.best("subghz", listOf(phone, bare)).device)
        assertEquals("wifi_board", Capabilities.capOf("wifi:ab", board.caps))
        assertEquals("wifi", Capabilities.capOf("wifi:ab", phone.caps))
    }
}

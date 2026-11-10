package network.hone.app.telemetry

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class SensorSummaryWireTest {
    private val cell = "89283082803ffff"

    @Test fun messageIsContractOrderWithSortedMetrics() {
        assertEquals(
            """{"chain_id":"hone","type":"SENSOR_SUMMARY","sensor_id":"coverage-ab12","owner":"shindevlin@ada9","batch_hash":"","cell":"89283082803ffff","window":1993120,"metrics":{"ble.crowd":20,"wifi.ap_count":27,"wifi.rssi_p50":-71}}""",
            SensorSummaryWire.message("hone", "coverage-ab12", "shindevlin@ada9", "", cell, 1993120,
                linkedMapOf("wifi.ap_count" to 27L, "wifi.rssi_p50" to -71L, "ble.crowd" to 20L)))
    }

    @Test fun refusesBadCellAndMetricNames() {
        try { SensorSummaryWire.message("hone", "s", "o", "", "87283082bffffff0", 1, mapOf("a" to 1L)); fail() } catch (_: IllegalArgumentException) {}
        try { SensorSummaryWire.message("hone", "s", "o", "", cell, 1, mapOf("Wifi.AP" to 1L)); fail() } catch (_: IllegalArgumentException) {}
        try { SensorSummaryWire.message("hone", "s", "o", "", cell, 1, emptyMap()); fail() } catch (_: IllegalArgumentException) {}
    }

    @Test fun bodySignatureVerifiesOverMessage() {
        val seed = ByteArray(32) { (it + 9).toByte() }   // test-only key
        val m = mapOf("wifi.ap_count" to 3L)
        val msg = SensorSummaryWire.message("hone", "coverage-x", "acct@pp", "", cell, 7, m)
        val sig = Canonical.sign(seed, msg)
        assertTrue(Canonical.verify(Canonical.pubOf(seed), msg, sig))
        val body = SensorSummaryWire.body("coverage-x", "acct@pp", "", cell, 7, m, Canonical.hex(sig))
        assertTrue(body.contains("\"metrics\":{\"wifi.ap_count\":3}") && body.contains("\"window\":7"))
    }

    @Test fun resolutionComesFromTheCellId() {
        assertEquals(9, SensorSummaryWire.res("89283082803ffff"))
        assertEquals(7, SensorSummaryWire.res("872830828ffffff"))
        assertEquals(-1, SensorSummaryWire.res("87283082bffffff0"))
        // res 7 and 8 are valid summary cells; finer or coarser are not.
        SensorSummaryWire.message("hone", "s", "o", "", "872830828ffffff", 1, mapOf("a" to 1L))
        SensorSummaryWire.message("hone", "s", "o", "", "8828308281fffff", 1, mapOf("a" to 1L))
        try { SensorSummaryWire.message("hone", "s", "o", "", "8a2830828107fff", 1, mapOf("a" to 1L)); fail() } catch (_: IllegalArgumentException) {}
        try { SensorSummaryWire.message("hone", "s", "o", "", "862830827ffffff", 1, mapOf("a" to 1L)); fail() } catch (_: IllegalArgumentException) {}
    }

    @Test fun everythingGoesOnChainOnlyThePlaceIsCoarsened() {
        // No home known: res 7, whatever the fix.
        assertEquals(7, SensorSummaryWire.chainRes(11, homeKnown = false, nearHome = false, atHome = false))
        // At or near home, or on the home Wi-Fi: res 7.
        assertEquals(7, SensorSummaryWire.chainRes(11, homeKnown = true, nearHome = true, atHome = false))
        assertEquals(7, SensorSummaryWire.chainRes(9, homeKnown = true, nearHome = false, atHome = true))
        // Away from home: the row's own resolution, capped at 9.
        assertEquals(9, SensorSummaryWire.chainRes(11, homeKnown = true, nearHome = false, atHome = false))
        assertEquals(8, SensorSummaryWire.chainRes(8, homeKnown = true, nearHome = false, atHome = false))
        assertEquals(7, SensorSummaryWire.chainRes(7, homeKnown = true, nearHome = false, atHome = false))
        // Only rows coarser than res 7 stay map only.
        assertEquals(null, SensorSummaryWire.chainRes(6, homeKnown = true, nearHome = false, atHome = false))
        assertEquals(null, SensorSummaryWire.chainRes(5, homeKnown = false, nearHome = false, atHome = false))
    }

    @Test fun coarseGateReadsTheNode() {
        val none = -1L to 99
        assertEquals(false, CoverageSubmitter.coarseActive(100, listOf(none)))                 // gate not announced
        assertEquals(false, CoverageSubmitter.coarseActive(100, listOf(101L to 99)))           // announced, not reached
        assertEquals(true, CoverageSubmitter.coarseActive(101, listOf(101L to 99)))
        assertEquals(true, CoverageSubmitter.coarseActive(5, listOf(none, -1L to 7)))         // min res reported
        assertEquals(false, CoverageSubmitter.coarseActive(5, listOf(-1L to 9)))
    }
}

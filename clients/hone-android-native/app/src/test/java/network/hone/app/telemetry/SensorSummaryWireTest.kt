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
}

package network.hone.app.data

import network.hone.app.telemetry.SensorSummaryWire
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Device-log fields on the wire (hone-node tx.rs with_device_log: appended last, only when present). */
class SyncWireTest {
    private val batch = SensorReadingBatch("phone-gnss", "josh", "abc123", 42, "continuous")

    @Test fun commitMessageAndBody() {
        val legacy = "{\"chain_id\":\"hone\",\"type\":\"SENSOR_DATA_COMMIT\",\"sensor_id\":\"phone-gnss\",\"owner\":\"josh\",\"batch_hash\":\"abc123\",\"reading_count\":42,\"sensor_type\":\"continuous\",\"signed_by\":\"josh\"}"
        assertEquals(legacy, SensorWire.sensorDataCommit("hone", batch))
        assertEquals(legacy.dropLast(1) + ",\"seq\":1}", SensorWire.sensorDataCommit("hone", batch, 1, null))
        assertEquals(legacy.dropLast(1) + ",\"seq\":8,\"prev_head\":\"ab\"}", SensorWire.sensorDataCommit("hone", batch, 8, "ab"))
        val b = JSONObject(SensorWire.commitBody(batch, "sig", 8, "ab"))
        assertEquals(8L, b.getLong("seq")); assertEquals("ab", b.getString("prev_head"))
        val first = JSONObject(SensorWire.commitBody(batch, "sig", 1, null))
        assertFalse(first.has("prev_head"))
        assertFalse(JSONObject(SensorWire.commitBody(batch, "sig")).has("seq"))
    }

    @Test fun summaryMessageAndBody() {
        val m = mapOf("wifi.ap_count" to 3L)
        val legacy = SensorSummaryWire.message("hone", "coverage-ab", "josh@ff", "", "896d75d960bffff", 7, m)
        assertTrue(legacy.endsWith("\"metrics\":{\"wifi.ap_count\":3}}"))
        assertEquals(legacy.dropLast(1) + ",\"seq\":4,\"prev_head\":\"cd\"}",
            SensorSummaryWire.message("hone", "coverage-ab", "josh@ff", "", "896d75d960bffff", 7, m, 4, "cd"))
        val b = JSONObject(SensorSummaryWire.body("coverage-ab", "josh@ff", "", "896d75d960bffff", 7, m, "sig", 4, "cd"))
        assertEquals(4L, b.getLong("seq")); assertEquals("cd", b.getString("prev_head"))
        assertFalse(JSONObject(SensorSummaryWire.body("coverage-ab", "josh@ff", "", "896d75d960bffff", 7, m, "sig")).has("seq"))
    }

    @Test fun clientGoesThroughTheEngine() {
        var sentBody = ""
        val transport = SensorTransport { _, body -> sentBody = body; SensorHttpResponse(200, "{\"accepted\":true,\"hash\":\"${"0f".repeat(32)}\"}") }
        val store = object : DeviceLogStore {
            val m = HashMap<String, DeviceLog.State>()
            override fun load(key: String) = m[key] ?: DeviceLog.State()
            override fun save(key: String, st: DeviceLog.State) { m[key] = st }
            override fun all() = m.toMap()
        }
        val engine = DeviceSyncEngine(store, { HeadFetch.Active(null) }, { _, _ -> })
        val client = SensorCommitClient("hone", transport, engine)
        val signed = ArrayList<String>()
        assertTrue(client.postReading(batch) { msg -> signed.add(String(msg)); byteArrayOf(1) }.accepted)
        assertTrue(signed[0].endsWith(",\"seq\":1}"))
        assertTrue(client.postReading(batch) { msg -> signed.add(String(msg)); byteArrayOf(1) }.accepted)
        assertTrue(signed[1].endsWith(",\"seq\":2,\"prev_head\":\"${"0f".repeat(32)}\"}"))
        assertEquals(2L, JSONObject(sentBody).getLong("seq"))
    }
}

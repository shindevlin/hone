package network.hone.app.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SensorCommitClientTest {
    private class CaptureTransport : SensorTransport {
        var path = ""
        var body = ""
        override fun post(path: String, jsonBody: String): SensorHttpResponse {
            this.path = path; body = jsonBody
            return SensorHttpResponse(200, "{\"accepted\":true}")
        }
    }

    private val fixedSigner = SensorSigner { message -> byteArrayOf(message.size.toByte(), 0x2a) }

    @Test fun canonicalCommitWireFormatMatchesRustInsertionOrderLock() {
        val batch = SensorReadingBatch("phone-gnss", "josh", "abc123", 42, "continuous")
        assertEquals(
            "{\"chain_id\":\"hone\",\"type\":\"SENSOR_DATA_COMMIT\",\"sensor_id\":\"phone-gnss\",\"owner\":\"josh\",\"batch_hash\":\"abc123\",\"reading_count\":42,\"sensor_type\":\"continuous\",\"signed_by\":\"josh\"}",
            SensorWire.sensorDataCommit("hone", batch),
        )
    }

    @Test fun enrolledDevicePathPostsSignedCommitOverHttpContract() {
        val transport = CaptureTransport()
        val client = SensorCommitClient("hone", transport)
        val response = client.postReading(
            SensorReadingBatch("phone-gnss", "josh", "abc123", 42, "continuous", 14.5),
            fixedSigner,
        )
        assertTrue(response.accepted)
        assertEquals("/api/sensor/commit", transport.path)
        assertTrue(transport.body.contains("\"signature\":\"c12a\""))
        assertTrue(transport.body.contains("\"value\":14.5"))
    }

    @Test fun ownerPostingFallbackUsesTheSameCommitWireContract() {
        val transport = CaptureTransport()
        val client = SensorCommitClient("hone", transport)
        client.postReading(SensorReadingBatch("phone-self", "josh", "ff", 1, "sampled"), fixedSigner)
        assertEquals("/api/sensor/commit", transport.path)
        assertTrue(transport.body.contains("\"owner\":\"josh\""))
        assertTrue(transport.body.contains("\"signature\":"))
    }

    @Test fun enrollmentAndRevocationUseOwnerSignedEndpoint() {
        val transport = CaptureTransport()
        val client = SensorCommitClient("hone", transport)
        client.enrollDevice("phone-gnss", "josh", "ab".repeat(32), fixedSigner)
        assertEquals("/api/sensor/key-register", transport.path)
        assertTrue(transport.body.contains("\"device_pubkey\":\"${"ab".repeat(32)}\""))
        client.revokeDevice("phone-gnss", "josh", fixedSigner)
        assertTrue(transport.body.contains("\"device_pubkey\":\"\""))
    }
}

package network.hone.app.data

import org.junit.Assert.assertEquals
import org.junit.Test
import java.security.PublicKey

class SensorSubmissionCoordinatorTest {
    @Test fun `device enrollment and commit use distinct signers without retaining owner key`() {
        val posted = mutableListOf<Pair<String, String>>()
        val coordinator = SensorSubmissionCoordinator(SensorCommitClient("hone") { path, body ->
            posted += path to body
            SensorHttpResponse(200, "{\"accepted\":true}")
        })
        val ownerSigner = SensorSigner { byteArrayOf(0x11) }
        val device = DeviceSensorIdentity("ab".repeat(32), SensorSigner { byteArrayOf(0x22) })
        val batch = SensorReadingBatch("phone-1", "josh", "cd".repeat(32), 1, "pressure")

        coordinator.enrollDevice("phone-1", "josh", device, ownerSigner)
        coordinator.submitWithEnrolledDevice(batch, device)

        assertEquals("/api/sensor/key-register", posted[0].first)
        assertEquals("11", Regex("\\\"signature\\\":\\\"([^\\\"]+)").find(posted[0].second)!!.groupValues[1])
        assertEquals("/api/sensor/commit", posted[1].first)
        assertEquals("22", Regex("\\\"signature\\\":\\\"([^\\\"]+)").find(posted[1].second)!!.groupValues[1])
    }

    @Test fun `owner fallback signs the same commit route`() {
        var body = ""
        val coordinator = SensorSubmissionCoordinator(SensorCommitClient("hone") { _, value ->
            body = value
            SensorHttpResponse(200, "{\"accepted\":true}")
        })
        coordinator.submitWithOwnerFallback(
            SensorReadingBatch("phone-2", "josh", "ef".repeat(32), 2, "motion"),
            SensorSigner { byteArrayOf(0x33) },
        )
        assertEquals("33", Regex("\\\"signature\\\":\\\"([^\\\"]+)").find(body)!!.groupValues[1])
    }

    @Test fun `x509 decoder accepts only the exact Ed25519 wrapper`() {
        val raw = ByteArray(32) { it.toByte() }
        val prefix = byteArrayOf(0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00)
        val key = object : PublicKey {
            override fun getAlgorithm() = "Ed25519"
            override fun getFormat() = "X.509"
            override fun getEncoded() = prefix + raw
        }
        assertEquals(raw.toList(), AndroidSensorKeyStore.rawEd25519PublicKey(key).toList())
    }

    @Test(expected = IllegalArgumentException::class)
    fun `x509 decoder rejects a differently wrapped key`() {
        val key = object : PublicKey {
            override fun getAlgorithm() = "EC"
            override fun getFormat() = "X.509"
            override fun getEncoded() = ByteArray(44)
        }
        AndroidSensorKeyStore.rawEd25519PublicKey(key)
    }

    @Test(expected = IllegalArgumentException::class)
    fun `device identity rejects malformed public key before enrollment`() {
        DeviceSensorIdentity("not-a-public-key", SensorSigner { byteArrayOf() })
    }
}

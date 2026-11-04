package network.hone.app.data

import org.junit.Assert.assertEquals
import org.junit.Test

class SensorNodeSettingsTest {
    @Test fun `settings require explicit values and derive channel sensor id`() {
        val settings = SensorNodeSettings("test-chain", "test-owner", "http://127.0.0.1:4242", "phone-a")
        assertEquals("phone-a-baro", settings.sensorId("baro"))
    }

    @Test(expected = IllegalArgumentException::class)
    fun `owner has no implicit default`() {
        SensorNodeSettings("test-chain", "", "http://127.0.0.1:4242", "phone-a")
    }

    @Test(expected = IllegalArgumentException::class)
    fun `node URL must be explicit and absolute`() {
        SensorNodeSettings("test-chain", "test-owner", "grouchly:4242", "phone-a")
    }
}

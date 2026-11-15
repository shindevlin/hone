package network.hone.app.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.security.MessageDigest

class PhoneSensorIdTest {
    private val keyA = "11".repeat(32)
    private val keyB = "22".repeat(32)

    @Test fun newFormIsModelPlusKeyHash() {
        val h = MessageDigest.getInstance("SHA-256").digest(ByteArray(32) { 0x11 }).joinToString("") { "%02x".format(it) }
        assertEquals("pixel-8-${h.take(8)}", PhoneSensorId.prefix("Pixel 8", keyA))
        assertEquals(PhoneSensorId.prefix("Pixel 8", keyA), PhoneSensorId.prefix("Pixel 8", keyA))   // stable
        assertTrue(Regex("^moto-g-power-5g-2024-[0-9a-f]{8}$").matches(PhoneSensorId.prefix("moto g power 5G (2024)", keyA)))
        assertTrue(PhoneSensorId.prefix("???", keyA).startsWith("phone-"))
    }

    @Test fun twoPhonesOfOneModelDiffer() {
        assertNotEquals(PhoneSensorId.prefix("moto g power", keyA), PhoneSensorId.prefix("moto g power", keyB))
    }

    @Test fun savedPrefixIsKept() {
        // phones whose sensors are already enrolled (e.g. "moto-gpower", "pixel-8") keep their ids
        assertEquals("moto-gpower", PhoneSensorId.resolve("moto-gpower", "moto g power") { keyA })
        assertEquals("pixel-8", PhoneSensorId.resolve(" pixel-8 ", "Pixel 8") { error("key not needed") })
        // new install: nothing saved
        assertEquals(PhoneSensorId.prefix("Pixel 8", keyA), PhoneSensorId.resolve(null, "Pixel 8") { keyA })
        assertEquals(PhoneSensorId.prefix("Pixel 8", keyA), PhoneSensorId.resolve("", "Pixel 8") { keyA })
        // no key at all (keystore and seat unavailable): the old form, never a crash
        assertEquals("pixel-8", PhoneSensorId.resolve(null, "Pixel 8") { null })
        assertEquals("pixel-8", PhoneSensorId.resolve(null, "Pixel 8") { error("keystore down") })
    }
}

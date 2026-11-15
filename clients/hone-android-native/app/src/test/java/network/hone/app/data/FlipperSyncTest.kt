package network.hone.app.data

import org.bouncycastle.crypto.params.Ed25519PrivateKeyParameters
import org.bouncycastle.crypto.signers.Ed25519Signer
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

class FlipperSyncTest {
    private val dev = FlipperSync.unhex("0011223344556677")

    /** Same input and bytes as the firmware's scripts/hone_sync_test.c (test_wire). */
    @Test fun headFileMatchesFirmwareGolden() {
        val f = FlipperSync.headFile(FlipperSync.F_ON or FlipperSync.F_REANCHOR, dev, listOf(
            FlipperSync.Entry(0, DeviceLog.Head(1234, "ab".repeat(32))),
            FlipperSync.Entry(5, null)))
        assertEquals(16 + 2 * 38 + 4, f.size)
        val head = byteArrayOf('H'.code.toByte(), 'S'.code.toByte(), 'H'.code.toByte(), '1'.code.toByte(), 3, 2, 0, 0,
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0, 1, 0xd2.toByte(), 0x04, 0, 0)
        assertArrayEquals(head, f.copyOfRange(0, 22))
        assertArrayEquals(byteArrayOf(0x67, 0x5c, 0xec.toByte(), 0xf5.toByte()), f.copyOfRange(f.size - 4, f.size))
    }

    @Test fun sCommitRoundTripAndSignature() {
        val sk = Ed25519PrivateKeyParameters(ByteArray(32) { 7 }, 0)
        val pub = sk.generatePublicKey().encoded
        val devId = FlipperSync.hex(pub.copyOfRange(0, 8))
        val hash = ByteArray(32) { 0x11 }
        val prev = ByteArray(32) { 0x22 }
        val msg = FlipperSync.message("flipper-$devId-wifi", "josh", FlipperSync.hex(hash), 61, "continuous", 70001, FlipperSync.hex(prev))
        assertTrue(msg.endsWith(",\"signed_by\":\"josh\",\"seq\":70001,\"prev_head\":\"${"22".repeat(32)}\"}"))
        val sig = Ed25519Signer().run { init(true, sk); val b = msg.toByteArray(); update(b, 0, b.size); generateSignature() }
        val d = ByteBuffer.allocate(FlipperSync.S_LEN).order(ByteOrder.LITTLE_ENDIAN)
            .put(pub, 0, 8).put(1.toByte()).putShort(61.toShort()).put(hash).putInt(70001).put(1.toByte()).put(prev).put(sig).array()
        val c = FlipperSync.parseS(d)!!
        assertEquals("flipper-$devId-wifi", c.sensorId)
        assertEquals(61, c.count); assertEquals(70001L, c.seq); assertEquals("22".repeat(32), c.prevHead)
        val again = FlipperSync.message(c.sensorId, "josh", c.hash, c.count, FlipperSync.TYPES[c.group], c.seq, c.prevHead)
        assertEquals(msg, again)
        val body = FlipperSync.body(c, "josh", "relay")
        assertEquals(70001L, body.getLong("seq")); assertEquals("22".repeat(32), body.getString("prev_head"))
        // a first commit has no prev_head in the message nor in the body
        d[47] = 0
        val first = FlipperSync.parseS(d)!!
        assertNull(first.prevHead)
        assertTrue(!FlipperSync.body(first, "josh", "").has("prev_head"))
        assertTrue(FlipperSync.message(first.sensorId, "josh", first.hash, 61, "continuous", 70001, null).endsWith(",\"seq\":70001}"))
        assertNull(FlipperSync.parseS(d.copyOf(107)))
    }

    @Test fun legacyMessageUnchanged() {
        assertEquals(
            "{\"chain_id\":\"hone\",\"type\":\"SENSOR_DATA_COMMIT\",\"sensor_id\":\"flipper-0011223344556677-subghz\",\"owner\":\"josh\",\"batch_hash\":\"ab\",\"reading_count\":3,\"sensor_type\":\"continuous\",\"signed_by\":\"josh\"}",
            FlipperSync.message("flipper-0011223344556677-subghz", "josh", "ab", 3, "continuous", null, null))
    }

    @Test fun bondedAddressFromBeaconAddress() {
        assertEquals("80:E1:EE:7A:1B:3F", FlipperSync.mainAddress("80:e1:11:7a:1b:c0"))
        assertNull(FlipperSync.mainAddress("nope"))
    }
}

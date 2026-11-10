package network.hone.app.data

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/** Active key vault crypto. Test-only keys; the outer Keystore layer is stood in for by a software AES key. */
class VaultCryptoTest {
    private val fast = VaultCrypto.Kdf(memKiB = 1024, iterations = 1, lanes = 1)
    private fun seed() = ByteArray(32) { (it * 13 + 5).toByte() }
    private fun pw(s: String = "correct horse battery") = s.toCharArray()
    private val sw: SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()
    private fun enc() = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, sw) }
    private fun dec(outer: ByteArray) = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE, sw, VaultCrypto.outerIv(outer)) }

    @Test fun roundTripWithDefaultArgon2idParameters() {
        val kdf = VaultCrypto.Kdf()
        assertEquals(64 * 1024, kdf.memKiB); assertEquals(3, kdf.iterations)
        val inner = VaultCrypto.seal(seed(), pw(), kdf)
        val outer = VaultCrypto.wrap(enc(), inner)
        val back = VaultCrypto.open(VaultCrypto.unwrap(dec(outer), outer), pw())
        assertArrayEquals(seed(), back)
        assertEquals(VaultCrypto.pubOf(seed()), VaultCrypto.pubOfBlob(inner))
    }

    @Test fun ciphertextHoldsNoPlaintextAndSaltIsRandom() {
        val a = VaultCrypto.seal(seed(), pw(), fast); val b = VaultCrypto.seal(seed(), pw(), fast)
        assertFalse(a.contentEquals(b))
        val s = seed()
        for (i in 0..a.size - 32) assertFalse(a.copyOfRange(i, i + 32).contentEquals(s))
    }

    @Test fun wrongPasswordIsRefused() {
        val inner = VaultCrypto.seal(seed(), pw(), fast)
        try { VaultCrypto.open(inner, pw("not the password")); fail() } catch (e: VaultCrypto.WrongPassword) {}
        try { VaultCrypto.signOnce(inner, pw("x"), "m".toByteArray()); fail() } catch (e: VaultCrypto.WrongPassword) {}
    }

    @Test fun tamperedOuterCiphertextOrTagIsDetected() {
        val outer = VaultCrypto.wrap(enc(), VaultCrypto.seal(seed(), pw(), fast))
        for (i in listOf(12, outer.size / 2, outer.size - 17)) {           // ciphertext bytes
            val t = outer.copyOf().also { it[i] = (it[i].toInt() xor 1).toByte() }
            try { VaultCrypto.unwrap(dec(t), t); fail("ct byte $i") } catch (e: VaultCrypto.Tampered) {}
        }
        for (i in outer.size - 16 until outer.size) {                      // GCM tag bytes
            val t = outer.copyOf().also { it[i] = (it[i].toInt() xor 0x80).toByte() }
            try { VaultCrypto.unwrap(dec(t), t); fail("tag byte $i") } catch (e: VaultCrypto.Tampered) {}
        }
    }

    @Test fun tamperedInnerCiphertextTagOrHeaderIsRefused() {
        val inner = VaultCrypto.seal(seed(), pw(), fast)
        val ctStart = inner.size - 48; val tagStart = inner.size - 16
        for (i in listOf(ctStart, ctStart + 31, tagStart, inner.size - 1, 20 /* salt */, 40 /* header pub */, 61 /* iv */)) {
            val t = inner.copyOf().also { it[i] = (it[i].toInt() xor 1).toByte() }
            try { VaultCrypto.open(t, pw()); fail("inner byte $i") }
            catch (e: VaultCrypto.WrongPassword) {} catch (e: VaultCrypto.Tampered) {}
        }
        val badMagic = inner.copyOf().also { it[0] = 'X'.code.toByte() }
        try { VaultCrypto.open(badMagic, pw()); fail() } catch (e: VaultCrypto.Tampered) {}
    }

    @Test fun plaintextSeedBufferIsZeroedAfterSigning() {
        val inner = VaultCrypto.seal(seed(), pw(), fast)
        var seen: ByteArray? = null
        val msg = "{\"chain_id\":\"hone\",\"type\":\"TRANSFER\"}".toByteArray()
        val (sig, pub) = VaultCrypto.signOnce(inner, pw(), msg) { seen = it }
        assertNotNull(seen)
        assertTrue("seed buffer must be all zero", seen!!.all { it == 0.toByte() })
        assertEquals(VaultCrypto.pubOf(seed()), pub)
        assertTrue(VaultCrypto.verify(pub, msg, sig))
        assertFalse(VaultCrypto.verify(pub, "other".toByteArray(), sig))
    }

    @Test fun seedIsZeroedEvenWhenSigningThrows() {
        // A wrong password never produces a buffer; a right one always ends zeroed (finally).
        val inner = VaultCrypto.seal(seed(), pw(), fast)
        var seen: ByteArray? = null
        try { VaultCrypto.signOnce(inner, pw(), "m".toByteArray()) { seen = it; throw IllegalStateException("boom") }; fail() }
        catch (e: IllegalStateException) {}
        assertTrue(seen!!.all { it == 0.toByte() })
    }

    @Test fun pubkeyMismatchIsRefused() {
        val s = seed()
        assertNull(VaultCrypto.checkAgainstChain(s, VaultCrypto.pubOf(s)))
        assertNull(VaultCrypto.checkAgainstChain(s, VaultCrypto.pubOf(s).uppercase()))
        val other = VaultCrypto.pubOf(ByteArray(32) { 1 })
        assertNotNull(VaultCrypto.checkAgainstChain(s, other))
        assertNotNull(VaultCrypto.checkAgainstChain(s, null))
        assertNotNull(VaultCrypto.checkAgainstChain(s, ""))
    }

    @Test fun seedHexIsStrict() {
        assertEquals(32, VaultCrypto.parseSeedHex("ab".repeat(32)).size)
        for (bad in listOf("ab".repeat(31), "zz".repeat(32), "ab".repeat(33), "")) {
            try { VaultCrypto.parseSeedHex(bad); fail(bad) } catch (e: IllegalArgumentException) {}
        }
    }

    @Test fun passwordBytesAreUtf8() {
        assertArrayEquals("pässwörd".toByteArray(Charsets.UTF_8), VaultCrypto.utf8("pässwörd".toCharArray()))
    }

    @Test fun backoffStartsAfterFiveFailuresAndGrows() {
        for (n in 0 until 5) assertEquals(0L, VaultBackoff.delayMs(n))
        assertEquals(30_000L, VaultBackoff.delayMs(5))
        assertEquals(60_000L, VaultBackoff.delayMs(6))
        assertEquals(120_000L, VaultBackoff.delayMs(7))
        assertEquals(3_600_000L, VaultBackoff.delayMs(20))
        assertEquals(3_600_000L, VaultBackoff.delayMs(Int.MAX_VALUE))
    }
}

package network.hone.app.sightings

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Test

/** Vectors from tools/hone_locshare.py (the Python reference), so both sides agree. */
class IdKeysTest {
    private val hide = ByteArray(32) { it.toByte() }
    private val dev = ByteArray(32) { (it + 32).toByte() }

    @Test fun schedule() {
        val seed = IdKeys.deviceSeed(hide, dev)
        assertEquals("4be589e7ea0f3020dea883578236a7379052ce525cf53b7ffe96a58ec7d555e2", seed.hex())
        val w = 1992930L
        assertEquals(24322, IdKeys.period(w))
        assertEquals(1992768L, IdKeys.periodWindows(24322).first)
        assertEquals(1995647L, IdKeys.periodWindows(24322).last)
        val share = IdKeys.shareKey(seed, 24322)
        assertEquals("eb4d000b6e98f96a4571104ac4e8ca2b1d4c03272c0ad839b68e82b022d99b92", share.hex())
        val k = IdKeys.windowKey(share, w)
        assertEquals("56eab8f5e50010dfd3a165a059f62aa086b88d473b7a27450578f664a81c6740", k.pk.hex())
        assertEquals("7298b5bfed0b71600cff936837e9fd00", k.lid.hex())
        assertEquals("58be", k.tag.hex())
        // A blob sealed by Python opens here.
        val blob = ("13be4feaeaf204c7fd3358fc9c00721881d174278128227ec674f37f7fe97b6d090909090909090909090909" +
            "899cb3b4779077c48805e3a0d70e1215b460a6bf22").unhex()
        assertEquals("hello", String(IdKeys.open(k.sk, k.pk, "hone-sighting-v1", blob)!!))
        // And round-trips with aad; wrong aad fails.
        val b2 = IdKeys.seal(k.pk, "x", "hi".toByteArray(), aad = k.lid)
        assertArrayEquals("hi".toByteArray(), IdKeys.open(k.sk, k.pk, "x", b2, aad = k.lid))
        assertEquals(null, IdKeys.open(k.sk, k.pk, "x", b2))
    }

    @Test fun memoConversion() {
        val memo = ByteArray(32) { 5 }
        assertEquals("6e7a1cdd29b0b78fd13af4c5598feff4ef2a97166e3ca6f2e4fbfccd80505bf1", IdKeys.ed25519Pub(memo).hex())
        val x = "c44e429251771ec76197c7a1f8ea289a18ca3dd7a7e102ba7cc84df6b55cbe1a"
        assertEquals(x, IdKeys.edPubToX(IdKeys.ed25519Pub(memo)).hex())
        assertEquals(x, IdKeys.xPub(IdKeys.edPrivToX(memo)).hex())
    }

    @Test fun grantKeyName() {
        assertEquals("ls1-f2c03fe29d653146fbb800b540019244", FindMy.grantKeyName("josh", "ab".repeat(32)))
    }
}

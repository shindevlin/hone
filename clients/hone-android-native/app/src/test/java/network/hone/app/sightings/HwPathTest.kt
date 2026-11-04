package network.hone.app.sightings

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

/** The hardware path (Keystore HMAC over the wrapped PRK) must give exactly the software
 *  path's keys and lids at generation 0, with a test hide key. */
class HwPathTest {
    private val hide = ByteArray(32) { (it * 13 + 5).toByte() }          // test-only key
    private val dev = ByteArray(32) { (0xa0 + it).toByte() }

    @Test fun gen0IsTodaysSeed() {
        assertArrayEquals(IdKeys.deviceSeed(hide, dev), IdKeys.deviceSeed(hide, dev, 0))
        assertFalse(IdKeys.deviceSeed(hide, dev).contentEquals(IdKeys.deviceSeed(hide, dev, 1)))
    }

    @Test fun prkExpandEqualsHkdfAndLidsMatch() {
        val seed = IdKeys.deviceSeed(hide, dev, 0)
        val prk = IdKeys.seedPrk(seed)
        val w0 = IdKeys.window(1_793_765_000_000L)
        for (w in w0 - 200..w0 + 3000 step 37) {
            val p = IdKeys.period(w)
            val hw = IdKeys.shareKeyFromPrk(prk, p)
            assertArrayEquals(IdKeys.shareKey(seed, p), hw)
            assertEquals(IdKeys.windowKeyFromSeed(seed, w).lid.hex(), IdKeys.windowKey(hw, w).lid.hex())
        }
    }
}

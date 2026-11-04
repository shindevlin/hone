package network.hone.app.birds

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PublishRuleTest {
    @Test fun currentRuleIsSeventyOrTwice() {
        assertFalse(BirdsSync.publishable(69.9, 1, true, tiered = false))
        assertTrue(BirdsSync.publishable(70.0, 1, null, tiered = false))
        assertTrue(BirdsSync.publishable(55.0, 2, true, tiered = false))
    }

    @Test fun tieredRule() {
        assertTrue(BirdsSync.publishable(60.0, 1, true, tiered = true))
        assertFalse(BirdsSync.publishable(59.9, 1, true, tiered = true))
        assertFalse(BirdsSync.publishable(65.0, 1, null, tiered = true))
        assertTrue(BirdsSync.publishable(70.0, 1, null, tiered = true))
        assertFalse(BirdsSync.publishable(84.0, 1, false, tiered = true))
        assertTrue(BirdsSync.publishable(85.0, 1, false, tiered = true))
        assertTrue(BirdsSync.publishable(54.0, 2, true, tiered = true))
    }
}

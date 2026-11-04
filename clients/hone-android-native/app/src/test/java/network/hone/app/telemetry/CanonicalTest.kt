package network.hone.app.telemetry

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class CanonicalTest {
    @Test fun sortsKeysAndHasNoWhitespace() {
        val m = linkedMapOf("z" to 1, "a" to mapOf("y" to true, "b" to null), "m" to listOf(3L, "x"))
        assertEquals("""{"a":{"b":null,"y":true},"m":[3,"x"],"z":1}""", Canonical.encode(m))
    }

    @Test fun escapesLikePython() {
        // python: json.dumps({"s": "a\"b\\c\nd\u0001é"}, sort_keys=True, separators=(",",":"), ensure_ascii=False)
        assertEquals("{\"s\":\"a\\\"b\\\\c\\nd\\u0001é\"}", Canonical.encode(mapOf("s" to "a\"b\\c\nd\u0001é")))
    }

    @Test fun refusesFractionalFloats() {
        assertEquals("""{"x":2}""", Canonical.encode(mapOf("x" to 2.0)))
        try { Canonical.encode(mapOf("x" to 2.5)); fail("floats must be refused") } catch (_: IllegalArgumentException) {}
    }

    @Test fun envelopeVerifies() {
        val seed = ByteArray(32) { (it * 3 + 5).toByte() }   // test-only key
        val payload = Canonical.encode(mapOf("account" to "testacct", "v" to 1))
        val env = Canonical.envelope(payload, "seat", seed)
        assertEquals("seat", env["key"])
        val pub = Canonical.unhex(env["pub"] as String)
        assertTrue(Canonical.verify(pub, payload, Canonical.unhex(env["sig"] as String)))
        assertFalse(Canonical.verify(pub, payload + " ", Canonical.unhex(env["sig"] as String)))
    }

    @Test fun envelopeIsItselfCanonical() {
        val seed = ByteArray(32) { 1 }
        val s = Canonical.encode(Canonical.envelope("{}", "posting", seed))
        assertTrue(s.startsWith("{\"key\":\"posting\",\"payload\":\"{}\",\"pub\":\""))
    }
}

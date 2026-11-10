package network.hone.app.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Pinned to the node's generic canonical_signing_message arm, output of hone-types at agent-key 85960b8a. */
class AgentKeyMessageTest {
    @Test fun setAgentKeyMatchesNode() {
        assertEquals(
            """{"chain_id":"hone-mainnet","entry":"{\"type\":\"SET_AGENT_KEY\",\"account\":\"josh\",\"agent_pubkey\":\"abababababababababababababababababababababababababababababababab\",\"cap\":1000000000000,\"nonce\":7,\"epoch\":123456,\"signed_by\":\"josh\"}"}""",
            AgentKeyMessage.set("hone-mainnet", "josh", "ab".repeat(32), 1_000_000_000_000, 7, 123456))
    }

    @Test fun revokeAgentKeyMatchesNode() {
        assertEquals(
            """{"chain_id":"hone-mainnet","entry":"{\"type\":\"REVOKE_AGENT_KEY\",\"account\":\"josh\",\"nonce\":8,\"epoch\":123457,\"signed_by\":\"josh\"}"}""",
            AgentKeyMessage.revoke("hone-mainnet", "josh", 8, 123457))
    }

    @Test fun capIsOneHundredHoneInHunits() {
        assertEquals(10_000_000_000L, WalletChain.HUNITS_PER_HONE)          // hone-types HUNITS_PER_HONE
        assertEquals(1_000_000_000_000L, AgentKey.CAP_HUNITS)               // AGENT_DEFAULT_CAP_HUNITS
        assertEquals("100.0 HONE", WalletChain.fmt(AgentKey.CAP_HUNITS))
        assertEquals(AgentKey.CAP_HUNITS, WalletChain.parseHone("100"))
    }

    @Test fun activeKeySignatureOverSetAgentKeyVerifies() {
        val fast = VaultCrypto.Kdf(memKiB = 1024, iterations = 1)
        val seed = ByteArray(32) { (it + 40).toByte() }                    // test-only key
        val inner = VaultCrypto.seal(seed, "a long password".toCharArray(), fast)
        val msg = AgentKeyMessage.set("hone", "josh", "cd".repeat(32), AgentKey.CAP_HUNITS, 1, 2).toByteArray()
        val (sig, pub) = VaultCrypto.signOnce(inner, "a long password".toCharArray(), msg)
        assertTrue(VaultCrypto.verify(pub, msg, sig))
    }

    @Test fun scopeTextIsExact() {
        assertEquals("Agent can: register and stake on josh's own devices, up to 100 HONE. Agent cannot: send tokens or change keys.", AgentKey.SCOPE_TEXT)
    }
}

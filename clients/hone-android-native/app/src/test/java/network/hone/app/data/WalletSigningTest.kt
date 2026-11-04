package network.hone.app.data

import org.bouncycastle.crypto.params.Ed25519PrivateKeyParameters
import org.bouncycastle.crypto.signers.Ed25519Signer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Pins the wallet signing messages to the node's canonical_signing_message (tx.rs,
 *  serde_json preserve_order, compact). */
class WalletSigningTest {
    @Test fun transferMessageMatchesNodeOrder() {
        assertEquals(
            """{"chain_id":"hone","type":"TRANSFER","from":"shindevlin","to":"josh","amount":15000000000,"token":"HONE","nonce":10}""",
            CanonicalMessage.transfer("hone", "shindevlin", "josh", 15_000_000_000, "HONE", 10))
    }

    @Test fun roleStakeMessagesMatchNodeOrder() {
        assertEquals(
            """{"chain_id":"hone","type":"NODE_ROLE_STAKE","node":"shindevlin@ab","role":"clock","staker":"shindevlin","amount":10000000000,"nonce":11}""",
            CanonicalMessage.nodeRoleStake("hone", false, "shindevlin@ab", "clock", "shindevlin", 10_000_000_000, 11))
        assertEquals(
            """{"chain_id":"hone","type":"NODE_ROLE_UNSTAKE","node":"n","role":"storage","staker":"s","amount":1,"nonce":2}""",
            CanonicalMessage.nodeRoleStake("hone", true, "n", "storage", "s", 1, 2))
    }

    @Test fun signatureVerifiesWithPublicKey() {
        val seed = ByteArray(32) { (it * 7 + 1).toByte() }   // test-only key
        val sk = Ed25519PrivateKeyParameters(seed, 0)
        val msg = CanonicalMessage.transfer("hone", "a", "b", 1, "HONE", 1).toByteArray()
        val sig = Ed25519Signer().run { init(true, sk); update(msg, 0, msg.size); generateSignature() }
        val ok = Ed25519Signer().run { init(false, sk.generatePublicKey()); update(msg, 0, msg.size); verifySignature(sig) }
        assertTrue(ok)
        assertEquals(64, ActiveKeyVault.pubOf(seed).length)
    }

    @Test fun amountsAreExactHunits() {
        assertEquals(10_000_000_000L, WalletChain.parseHone("1"))
        assertEquals(15_000_000_000L, WalletChain.parseHone("1.5"))
        assertEquals(1L, WalletChain.parseHone("0.0000000001"))
        assertEquals(1_000_000_000L, WalletChain.parseHone(".1"))
        assertNull(WalletChain.parseHone("0"))
        assertNull(WalletChain.parseHone("-1"))
        assertNull(WalletChain.parseHone("1.00000000001"))
        assertNull(WalletChain.parseHone("1e3"))
        assertEquals("1.5 HONE", WalletChain.fmt(15_000_000_000L))
        assertEquals("0.0000001 HONE", WalletChain.fmt(1_000L))
        assertTrue(WalletChain.validName("shindevlin@ada95c12956497f29b29dd768acc0f862fdc5fc54c1ac59b6b272dac6a12a1da"))
    }
}
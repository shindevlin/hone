package network.hone.app.data

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import org.bouncycastle.math.ec.rfc8032.Ed25519
import org.json.JSONObject
import java.io.File
import java.math.BigInteger
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/**
 * Scoped agent key (chain branch agent-key 85960b8a, gate AGENT_KEY_EPOCH).
 * The agent may register and stake on the account's own devices up to [CAP_HUNITS]; it can
 * never send tokens or change keys. SET_AGENT_KEY is signed by the active key;
 * REVOKE_AGENT_KEY by the active key (or the agent key itself, not offered here).
 */
object AgentKey {
    /** 100 HONE in hunits (1 HONE = 10^10 hunits) = hone-types AGENT_DEFAULT_CAP_HUNITS. */
    const val CAP_HUNITS = 100L * WalletChain.HUNITS_PER_HONE
    const val SCOPE_TEXT = "Agent can: register and stake on josh's own devices, up to 100 HONE. Agent cannot: send tokens or change keys."
    private val PUB = Regex("[0-9a-f]{64}")
    fun validPub(s: String) = PUB.matches(s)

    /**
     * What the chain says. [readable] false = the node has no agent-key read yet (older node).
     * The gate counts as active only when the chain reports active_from_epoch <= epoch.
     */
    data class Status(
        val readable: Boolean,
        val activeFrom: BigInteger?,
        val epoch: Long?,
        val pubkey: String?,
        val cap: Long?,
        val staked: Long?,
        val stakes: List<WalletChain.Stake>?,
    ) {
        val gateActive: Boolean get() = activeFrom != null && epoch != null && activeFrom <= BigInteger.valueOf(epoch)
    }

    /**
     * GET /api/account/<account>/agent-key, expected shape:
     * {"active_from_epoch": n, "epoch": n, "agent_key": {"pubkey","cap"} | null, "staked": n,
     *  "stakes": [{"node","role","amount"}]}. Falls back to an "agent_key" object on /api/account/<a>.
     */
    fun status(account: String): Status {
        val r = ChainHttp.getWithCode("/api/account/$account/agent-key")
        val o = if (r != null && r.first in 200..299 && r.second != null) runCatching { JSONObject(r.second!!) }.getOrNull() else null
        val src = o ?: ChainHttp.getWithCode("/api/account/$account")?.second
            ?.let { runCatching { JSONObject(it) }.getOrNull() }?.takeIf { it.has("agent_key") || it.has("agent_key_active_from_epoch") }
            ?: return Status(false, null, null, null, null, null, null)
        fun big(k: String) = src.opt(k)?.toString()?.takeIf { it.matches(Regex("\\d+")) }?.let(::BigInteger)
        val ak = src.optJSONObject("agent_key")
        val stakes = src.optJSONArray("stakes")?.let { a -> (0 until a.length()).map { a.getJSONObject(it) }
            .map { WalletChain.Stake(it.optString("node"), it.optString("role"), account, it.optLong("amount")) } }
        return Status(true,
            big("active_from_epoch") ?: big("agent_key_active_from_epoch"),
            big("epoch")?.toLong() ?: runCatching { WalletChain.nodeInfo().epoch }.getOrNull(),
            ak?.optString("pubkey")?.takeIf { validPub(it) },
            ak?.optLong("cap")?.takeIf { it > 0 },
            src.optLong("staked", src.optLong("agent_staked", 0)),
            stakes)
    }

    fun submitSet(account: String, pub: String, cap: Long, nonce: Long, epoch: Long, sig: String): WalletChain.Submit =
        post("/api/account/agent-key/set", JSONObject().put("account", account).put("agent_pubkey", pub).put("cap", cap)
            .put("nonce", nonce).put("epoch", epoch).put("signed_by", account).put("signature", sig))

    fun submitRevoke(account: String, nonce: Long, epoch: Long, sig: String): WalletChain.Submit =
        post("/api/account/agent-key/revoke", JSONObject().put("account", account)
            .put("nonce", nonce).put("epoch", epoch).put("signed_by", account).put("signature", sig))

    private fun post(path: String, body: JSONObject): WalletChain.Submit {
        val (code, text) = ChainHttp.postJson(path, body.toString()) ?: return WalletChain.Submit(false, null, "no HONE node reachable")
        val o = runCatching { JSONObject(text) }.getOrNull() ?: return WalletChain.Submit(false, null, "HTTP $code")
        return WalletChain.Submit(o.optBoolean("accepted", false), o.optString("hash").takeIf { it.isNotEmpty() && it != "null" },
            o.optString("error").takeIf { it.isNotEmpty() && it != "null" })
    }

    // ── Agent key generated on this phone (kept under a Keystore key; never shown) ──
    private const val ALIAS = "hone.agent.v1"
    private fun file(ctx: Context) = File(ctx.filesDir, "hone-node/agent_key.enc")
    private fun pubFile(ctx: Context) = File(ctx.filesDir, "hone-node/agent_key.pub")
    fun phonePub(ctx: Context): String? = pubFile(ctx).takeIf { it.exists() && file(ctx).exists() }?.readText()?.trim()

    private fun key(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val spec = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256).build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run { init(spec); generateKey() }
    }

    /** New agent keypair on this phone; returns its public key. Replaces any earlier one. */
    fun generateOnPhone(ctx: Context): String {
        val seed = ByteArray(32).also { SecureRandom().nextBytes(it) }
        try {
            val pk = ByteArray(32).also { Ed25519.generatePublicKey(seed, 0, it, 0) }
            val blob = VaultCrypto.wrap(Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, key()) }, seed)
            val f = file(ctx); f.parentFile?.mkdirs()
            val tmp = File(f.parentFile, f.name + ".tmp"); tmp.writeBytes(blob); tmp.renameTo(f)
            val pub = pk.joinToString("") { "%02x".format(it) }
            pubFile(ctx).writeText(pub)
            return pub
        } finally { seed.fill(0) }
    }

    fun removePhoneKey(ctx: Context) {
        file(ctx).delete(); pubFile(ctx).delete()
        runCatching { KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.deleteEntry(ALIAS) }
    }
}

/**
 * Signing bytes for the agent-key entries. They have no explicit arm in tx.rs
 * canonical_signing_message, so the node uses the generic one:
 * {"chain_id":<id>,"entry":<serde_json::to_string(entry)>} where the entry is internally
 * tagged ("type" first) with fields in declaration order. Pinned in AgentKeyMessageTest
 * against output of hone-types at 85960b8a.
 */
object AgentKeyMessage {
    fun set(chainId: String, account: String, agentPub: String, cap: Long, nonce: Long, epoch: Long): String =
        outer(chainId, buildString {
            append("{\"type\":\"SET_AGENT_KEY\",\"account\":").append(q(account))
            append(",\"agent_pubkey\":").append(q(agentPub))
            append(",\"cap\":").append(cap)
            append(",\"nonce\":").append(nonce)
            append(",\"epoch\":").append(epoch)
            append(",\"signed_by\":").append(q(account)).append('}')
        })

    fun revoke(chainId: String, account: String, nonce: Long, epoch: Long): String =
        outer(chainId, buildString {
            append("{\"type\":\"REVOKE_AGENT_KEY\",\"account\":").append(q(account))
            append(",\"nonce\":").append(nonce)
            append(",\"epoch\":").append(epoch)
            append(",\"signed_by\":").append(q(account)).append('}')
        })

    private fun outer(chainId: String, entry: String) = "{\"chain_id\":${q(chainId)},\"entry\":${q(entry)}}"

    /** serde_json string encoding. */
    internal fun q(s: String): String = buildString {
        append('"')
        for (c in s) when (c) {
            '"' -> append("\\\""); '\\' -> append("\\\\")
            '\n' -> append("\\n"); '\r' -> append("\\r"); '\t' -> append("\\t")
            '\b' -> append("\\b"); '\u000c' -> append("\\f")
            else -> if (c < ' ') append("\\u%04x".format(c.code)) else append(c)
        }
        append('"')
    }
}

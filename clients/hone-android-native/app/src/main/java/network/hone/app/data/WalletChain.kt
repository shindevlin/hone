package network.hone.app.data

import org.json.JSONObject

/**
 * Chain reads and signed submissions for the wallet (Transfer, NodeRoleStake/Unstake).
 * Every call is blocking; run on Dispatchers.IO. Amounts are hunits (1 HONE = 10^10).
 */
object WalletChain {
    const val HUNITS_PER_HONE = 10_000_000_000L
    const val TOKEN = "HONE"
    const val WEIGHT_TRANSFER = 1L       // hone-types ENTRY_WEIGHT_MICRO
    const val WEIGHT_ROLE_STAKE = 3L     // ENTRY_WEIGHT_STANDARD
    val ROLES = listOf("clock", "storage", "service", "verifier", "sensor", "miner", "mempool", "decide")
    private val NAME = Regex("[a-z0-9][a-z0-9._-]{0,63}(@[0-9a-f]{64})?")

    data class Account(val name: String, val nonce: Long, val activePub: String?)
    data class NodeInfo(val chainId: String, val epoch: Long)
    data class Submit(val accepted: Boolean, val hash: String?, val error: String?)
    data class Stake(val node: String, val role: String, val staker: String, val amount: Long)

    fun validName(s: String) = NAME.matches(s)

    fun account(name: String): Account? {
        val (code, body) = ChainHttp.getWithCode("/api/account/$name") ?: error("no HONE node reachable")
        if (code == 404 || body == null) return null
        val o = JSONObject(body)
        return Account(o.getString("account"), o.optLong("nonce", 0),
            o.optJSONObject("keys")?.optString("active")?.takeIf { it.isNotEmpty() })
    }

    fun nodeInfo(): NodeInfo {
        val o = JSONObject(ChainHttp.get("/api/node/info") ?: error("no HONE node reachable"))
        return NodeInfo(o.optString("chain_id", "hone"), o.getLong("epoch"))
    }

    fun baseFee(): Long = runCatching {
        JSONObject(ChainHttp.get("/api/chain/fee-estimate")!!).getLong("base_fee")
    }.getOrDefault(10_000_000L)

    /** An account name, or a name resolved via /api/resolve/name. Returns the account or an error. */
    fun resolveRecipient(input: String): Result<String> = runCatching {
        val r = input.trim().removePrefix("@").lowercase()
        require(validName(r)) { "Not a valid account or name" }
        if (account(r) != null) return@runCatching r
        val res = ChainHttp.getWithCode("/api/resolve/name/$r")?.second?.let { JSONObject(it) }
        val acct = res?.optString("account")?.takeIf { it.isNotEmpty() && it != "null" }
            ?: error("No account or name \"$r\" on chain")
        require(account(acct) != null) { "\"$r\" points to $acct, which does not exist" }
        acct
    }

    fun submitTransfer(from: String, to: String, amount: Long, memo: String?, nonce: Long, sig: String): Submit {
        val body = JSONObject().put("from", from).put("to", to).put("amount", amount).put("token", TOKEN)
            .put("signed_by", from).put("nonce", nonce).put("signature", sig)
        if (!memo.isNullOrBlank()) body.put("memo", memo)
        return submit("/api/transfer", body)
    }

    fun submitRoleStake(unstake: Boolean, node: String, role: String, staker: String, amount: Long,
                        epoch: Long, nonce: Long, sig: String): Submit {
        val body = JSONObject().put("node", node).put("role", role).put("staker", staker).put("amount", amount)
            .put("epoch", epoch).put("nonce", nonce).put("signed_by", staker).put("signature", sig)
        return submit(if (unstake) "/api/node/role/unstake" else "/api/node/role/stake", body)
    }

    private fun submit(path: String, body: JSONObject): Submit {
        val (code, text) = ChainHttp.postJson(path, body.toString()) ?: return Submit(false, null, "no HONE node reachable")
        val o = runCatching { JSONObject(text) }.getOrNull() ?: return Submit(false, null, "HTTP $code")
        return Submit(o.optBoolean("accepted", false), o.optString("hash").takeIf { it.isNotEmpty() && it != "null" },
            o.optString("error").takeIf { it.isNotEmpty() && it != "null" })
    }

    /** (status, reason) from /api/entry/<hash>/status: committed / rejected / pending / unknown. */
    fun status(hash: String): Pair<String, String?> {
        val o = JSONObject(ChainHttp.get("/api/entry/$hash/status") ?: return "unreachable" to null)
        return o.optString("status", "unknown") to (o.optString("reason").takeIf { it.isNotEmpty() && it != "null" }
            ?: o.optString("error").takeIf { it.isNotEmpty() && it != "null" })
    }

    fun stakes(node: String, role: String?): List<Stake> {
        val path = "/api/node/role/stakes/$node" + (role?.let { "/$it" } ?: "")
        val o = JSONObject(ChainHttp.get(path) ?: return emptyList())
        fun parse(a: org.json.JSONArray?) = (0 until (a?.length() ?: 0)).map { a!!.getJSONObject(it) }
            .map { Stake(it.optString("node"), it.optString("role"), it.optString("staker"), it.optLong("amount")) }
        return if (role != null) parse(o.optJSONArray("backers"))
        else o.optJSONObject("stakes")?.let { s -> s.keys().asSequence().flatMap { parse(s.optJSONArray(it)).asSequence() }.toList() }.orEmpty()
    }

    /** "1.5" HONE -> hunits, exact (no floating point). Null when not a positive amount. */
    fun parseHone(s: String): Long? {
        val t = s.trim()
        if (!t.matches(Regex("\\d{1,9}(\\.\\d{0,10})?|\\.\\d{1,10}"))) return null
        val (i, f) = t.split('.').let { it[0].ifEmpty { "0" } to (it.getOrNull(1) ?: "") }
        val v = i.toLong() * HUNITS_PER_HONE + (f.padEnd(10, '0').toLong())
        return v.takeIf { it > 0 }
    }

    fun fmt(h: Long): String {
        val s = (h / HUNITS_PER_HONE).toString() + "." + (h % HUNITS_PER_HONE).toString().padStart(10, '0').trimEnd('0').ifEmpty { "0" }
        return "$s HONE"
    }
}
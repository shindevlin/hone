package network.hone.app.data

import android.content.Context
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL

/**
 * Read-only HTTP view of the live chain. Tries each known node in order and
 * sticks with the first that answers: grouchly on the LAN, nebra, beastly, then
 * grouchly over Tailscale for when the phone is away from home.
 */
object ChainHttp {
    val NODES = listOf(
        "http://192.168.68.72:4242",
        "http://192.168.68.75:4242",
        "http://192.168.68.79:4242",
        "http://100.90.146.17:4242",
    )
    @Volatile private var preferred = 0

    fun get(path: String): String? {
        for (i in NODES.indices) {
            val idx = (preferred + i) % NODES.size
            val body = runCatching {
                val c = URL(NODES[idx] + path).openConnection() as HttpURLConnection
                c.connectTimeout = 3_000
                c.readTimeout = 8_000
                try {
                    if (c.responseCode !in 200..299) null
                    else c.inputStream.bufferedReader().use { it.readText() }
                } finally { c.disconnect() }
            }.getOrNull()
            if (body != null) { preferred = idx; return body }
        }
        return null
    }

    fun balanceHunits(account: String): ULong? =
        get("/api/balance/$account")?.let { runCatching { JSONObject(it).getLong("hunits").toULong() }.getOrNull() }

    fun stakedHunits(account: String): ULong? =
        get("/api/staking/account/$account/positions")?.let { body ->
            runCatching {
                val arr = JSONObject(body).getJSONArray("positions")
                var sum = 0UL
                for (i in 0 until arr.length()) {
                    val p = arr.getJSONObject(i)
                    if (p.optString("staker") == account) sum += p.optLong("amount").toULong()
                }
                sum
            }.getOrNull()
        }

    fun history(account: String, limit: Int = 50): List<Tx>? =
        get("/api/account/$account/history?limit=$limit")?.let { body ->
            runCatching {
                val arr: JSONArray = JSONObject(body).getJSONArray("entries")
                (0 until arr.length()).map { i ->
                    val e = arr.getJSONObject(i)
                    val entry = e.optJSONObject("entry") ?: JSONObject()
                    val type = e.optString("type")
                    val from = entry.optString("from", "")
                    val to = entry.optString("to", "")
                    val out = from == account && to != account
                    val amount = listOf("amount", "sensor_amount", "gateway_amount")
                        .map { entry.optLong(it, 0) }.firstOrNull { it > 0 } ?: 0L
                    Tx(
                        id = "${e.optLong("epoch")}-$type-$i",
                        direction = if (out) TxDirection.OUT else TxDirection.IN,
                        counterparty = when {
                            out -> "@$to"
                            from.isNotEmpty() -> "@$from"
                            else -> type.lowercase().replace('_', '-')
                        },
                        amountHunits = amount.toULong(),
                        epoch = e.optLong("epoch").toULong(),
                        status = TxStatus.CONFIRMED,
                        memo = entry.optString("memo", "").takeIf { it.isNotBlank() && it != "null" },
                    )
                }
            }.getOrNull()
        }
}

/**
 * The phone's wallet, read from the chain. The account is the one this phone's
 * clock seat belongs to (hone-node/witness_account, e.g. "josh"): the seat's
 * rewards are paid to that account.
 *
 * Sending is NOT wired: the owner's keys do not live on this phone (the seat
 * signs with its own device key, which can only seal). A send has to be a
 * sign-request to the owner's wallet.
 */
class ChainWalletRepository(context: Context) : WalletRepository {
    private val account: String = runCatching {
        context.filesDir.resolve("hone-node/witness_account").readText().trim()
    }.getOrDefault("")

    private val _identity = MutableStateFlow(
        WalletIdentity(
            name = if (account.isEmpty()) "(no account)" else "@$account",
            address = account,
            balanceHunits = 0UL,
            delegatedHunits = 0UL,
        )
    )
    override val identity: StateFlow<WalletIdentity> = _identity.asStateFlow()

    private val _history = MutableStateFlow<List<Tx>>(emptyList())
    override val history: StateFlow<List<Tx>> = _history.asStateFlow()

    init {
        if (account.isNotEmpty()) {
            CoroutineScope(SupervisorJob() + Dispatchers.IO).launch {
                while (true) {
                    refresh()
                    delay(30_000)
                }
            }
        }
    }

    private fun refresh() {
        val bal = ChainHttp.balanceHunits(account)
        val staked = ChainHttp.stakedHunits(account)
        if (bal == null) {
            Log.w("ChainWallet", "no node reachable for $account")
            return
        }
        _identity.value = _identity.value.copy(
            balanceHunits = bal,
            delegatedHunits = staked ?: _identity.value.delegatedHunits,
        )
        ChainHttp.history(account)?.let { _history.value = it }
    }

    override fun isValidRecipient(recipient: String): Boolean {
        val r = recipient.trim().removePrefix("@")
        return r.length in 1..64 && r.all { it.isLetterOrDigit() || it == '-' || it == '_' || it == '.' }
    }

    override suspend fun send(recipient: String, amountHone: Double, memo: String?): SendResult =
        SendResult.Rejected("Sending from the phone is not wired yet — the owner's keys are not on this device")
}

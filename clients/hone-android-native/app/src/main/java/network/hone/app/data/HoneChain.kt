package network.hone.app.data

import org.json.JSONObject

/**
 * Chain reads for the HONE-mode screens (earnings, backing, seats, feature gates).
 * Blocking; call on Dispatchers.IO.
 */
object HoneChain {
    private const val ZERO = "0000000000000000000000000000000000000000000000000000000000000000"

    data class Position(val node: String, val role: String, val amount: Long)
    data class Claim(val cell: String, val metric: String, val window: Long, val status: String, val points: Long,
                     val claimedEpoch: Long, val settledEpoch: Long?)
    data class Usefulness(val provenPoints: Long, val queuedPoints: Long, val proven: Int, val pending: Int, val void: Int,
                          val activeFrom: Long, val claimWindowEpochs: Long, val claims: List<Claim>)

    /** Stakes this account placed, on its own nodes and on other people's. */
    fun positions(account: String): List<Position>? = ChainHttp.get("/api/staking/account/$account/positions")?.let { b ->
        runCatching {
            val a = JSONObject(b).getJSONArray("positions")
            (0 until a.length()).map { a.getJSONObject(it) }.filter { it.optString("staker") == account }
                .map { Position(it.optString("node"), it.optString("role"), it.optLong("amount")) }
        }.getOrNull()
    }

    fun isMine(account: String, node: String) = node == account || node.startsWith("$account@")

    fun usefulness(owner: String): Usefulness? = ChainHttp.get("/api/sensor/usefulness/$owner")?.let { b ->
        runCatching {
            val o = JSONObject(b)
            val c = o.optJSONObject("counts") ?: JSONObject()
            val arr = o.optJSONArray("claims")
            val claims = (0 until (arr?.length() ?: 0)).map { arr!!.getJSONObject(it) }.map {
                Claim(it.optString("cell"), it.optString("metric"), it.optLong("window"), it.optString("status"),
                    it.optLong("points"), it.optLong("claimed_epoch"),
                    if (it.isNull("settled_epoch")) null else it.optLong("settled_epoch"))
            }
            Usefulness(o.optLong("proven_points"), o.optLong("queued_points"), c.optInt("proven"), c.optInt("pending"),
                c.optInt("void"), o.optLong("active_from_epoch"), o.optLong("claim_window_epochs", 2880), claims)
        }.getOrNull()
    }

    /** GET /api/sensor/seat/:seat (a seat id "owner@pubkey" or a seat pubkey). */
    fun seat(seat: String): JSONObject? = ChainHttp.get("/api/sensor/seat/$seat")?.let { runCatching { JSONObject(it) }.getOrNull() }

    fun tip(): Long? = runCatching { WalletChain.nodeInfo().epoch }.getOrNull()

    /** Live state of a chain feature that is switched on at an activation epoch. Null = no node answered. */
    private fun gate(path: String, field: String): Boolean? {
        val (code, body) = ChainHttp.getWithCode(path) ?: return null
        if (code !in 200..299 || body == null) return false            // route not deployed or not public yet
        val from = runCatching { JSONObject(body).optLong(field, Long.MAX_VALUE) }.getOrDefault(Long.MAX_VALUE)
        if (from < 0 || from == Long.MAX_VALUE) return false             // u64::MAX = switched off
        val t = tip() ?: return null
        return from <= t
    }

    fun vouchersOn(): Boolean? = gate("/api/sponsor/voucher/$ZERO", "active_from_epoch")
    fun birdCardsOn(): Boolean? = gate("/api/bird/$ZERO", "active_from_epoch")
    fun seatsOn(): Boolean? = gate("/api/sensor/seat/$ZERO", "seat_register_from_epoch")
}

/** Plain words for chain names. Technical names stay in expert rows. */
object Plain {
    val ROLE = mapOf(
        "clock" to "Timekeeper", "decide" to "Checking panel", "sensor" to "Sensor", "storage" to "Storage",
        "service" to "Services", "verifier" to "Verifier", "miner" to "Miner", "mempool" to "Relay",
    )
    fun role(r: String) = ROLE[r] ?: r

    /** HONE amount for people: 2 decimals, or 4 for small rewards. */
    fun hone(h: Long): String = if (h != 0L && kotlin.math.abs(h) < 100_000_000L) "%,.4f".format(h / 1e10) else "%,.2f".format(h / 1e10)

    fun metric(m: String): String = when {
        m == "ble.crowd" -> "How busy Bluetooth is"
        m == "hone.devices" -> "HONE devices nearby"
        m == "mobile_ap" -> "Phone hotspots nearby"
        m == "wifi.ap_count" -> "Wi-Fi networks nearby"
        m == "wifi.rssi_max" -> "Strongest Wi-Fi signal"
        m == "wifi.rssi_p50" -> "Typical Wi-Fi signal"
        m.startsWith("wifi.band.") -> "Wi-Fi on " + m.removePrefix("wifi.band.").replace("g", " GHz")
        m.startsWith("subghz") -> "Radio signals (sub-GHz)"
        m.startsWith("ir") -> "Infrared signals"
        m.startsWith("sound") || m.startsWith("bird") -> "Bird sound"
        else -> m.replace('.', ' ').replace('_', ' ').replaceFirstChar { it.uppercase() }
    }

    /** Why a find is paid, waiting, or not paid. */
    fun reason(status: String, windowHours: Long): String = when (status) {
        "proven" -> "New for the map and matched by someone else's device. Counted as proven useful."
        "pending" -> "New for the map. Waiting for someone else's device to check it (up to about $windowHours h)."
        "void" -> "Nobody else confirmed it in time, so it earns nothing. Nothing is held against you."
        else -> status
    }

    fun entry(type: String, out: Boolean, other: String): String = when (type) {
        "transfer", "" -> if (out) "Sent to $other" else "From $other"
        "clock-reward" -> "Paid for keeping time"
        "storage-reward" -> "Paid for storing data"
        "mempool-reward" -> "Paid for relaying"
        "service-reward" -> "Paid for services"
        "miner-reward", "mine-reward" -> "Paid for mining"
        "sensor-reward" -> "Paid for finds"
        "decide-reward" -> "Paid for checking finds"
        else -> type.replace('-', ' ').replaceFirstChar { it.uppercase() }
    }
}

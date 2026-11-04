package network.hone.app.telemetry

/**
 * SENSOR_SUMMARY wire format (chain contract, 2026-11-04): compact JSON, keys in this
 * exact order, metric keys sorted, integer values; Ed25519 over the UTF-8 bytes.
 * Pure Kotlin so the byte-exact message is unit-tested.
 */
object SensorSummaryWire {
    val METRIC = Regex("^[a-z0-9._:]{1,40}$")
    val CELL9 = Regex("^8[0-9a-f]{14}$")
    const val MAX_METRICS = 32

    fun message(chainId: String, sensorId: String, owner: String, batchHash: String, cell: String,
                window: Long, metrics: Map<String, Long>): String {
        require(CELL9.matches(cell)) { "cell must be an H3 res-9 id" }
        require(metrics.size in 1..MAX_METRICS) { "1..$MAX_METRICS metrics" }
        require(metrics.keys.all { METRIC.matches(it) }) { "bad metric name" }
        val m = metrics.toSortedMap().entries.joinToString(",") { (k, v) -> "\"$k\":$v" }
        return "{\"chain_id\":${q(chainId)},\"type\":\"SENSOR_SUMMARY\",\"sensor_id\":${q(sensorId)}," +
            "\"owner\":${q(owner)},\"batch_hash\":${q(batchHash)},\"cell\":\"$cell\",\"window\":$window,\"metrics\":{$m}}"
    }

    /** POST body for /api/sensor/summary. */
    fun body(sensorId: String, owner: String, batchHash: String, cell: String, window: Long,
             metrics: Map<String, Long>, signatureHex: String): String =
        Canonical.encode(mapOf("sensor_id" to sensorId, "owner" to owner, "batch_hash" to batchHash, "cell" to cell,
            "window" to window, "metrics" to metrics.toSortedMap(), "signature" to signatureHex))

    private fun q(s: String) = Canonical.encode(s)
}

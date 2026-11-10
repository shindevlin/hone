package network.hone.app.telemetry

/**
 * SENSOR_SUMMARY wire format (chain contract, 2026-11-04; res 7-9 cells 2026-11-09): compact JSON, keys in this
 * exact order, metric keys sorted, integer values; Ed25519 over the UTF-8 bytes.
 * Pure Kotlin so the byte-exact message is unit-tested.
 */
object SensorSummaryWire {
    val METRIC = Regex("^[a-z0-9._:]{1,40}$")
    /** An H3 cell id as 15 hex digits; the second digit is its resolution. */
    val CELL = Regex("^8[0-9a-f]{14}$")
    const val MAX_METRICS = 32
    const val MIN_RES = 7      // ~5 km² (home, near home, no home known)
    const val MAX_RES = 9      // a block (~0.1 km²), the finest that goes on chain

    /** The H3 resolution encoded in [cell], or -1 if it is not a cell id. */
    fun res(cell: String): Int = if (CELL.matches(cell)) cell[1].digitToInt(16) else -1

    /**
     * Resolution a summary goes on chain at. Everything goes on chain; only the place is coarsened:
     * res 7 at or near home, on the home Wi-Fi, or when no home is known; elsewhere the row's own
     * resolution capped at 9. Null only for rows already coarser than res 7 (map only).
     */
    fun chainRes(rowRes: Int, homeKnown: Boolean, nearHome: Boolean, atHome: Boolean): Int? = when {
        rowRes < MIN_RES -> null
        !homeKnown || nearHome || atHome -> MIN_RES
        else -> minOf(rowRes, MAX_RES)
    }

    fun message(chainId: String, sensorId: String, owner: String, batchHash: String, cell: String,
                window: Long, metrics: Map<String, Long>): String {
        require(cell.isEmpty() || res(cell) in MIN_RES..MAX_RES) { "cell must be an H3 res $MIN_RES-$MAX_RES id, or empty (NULL_CELL gate)" }
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

package network.hone.app.data

/**
 * Builds the EXACT canonical signing message the node verifies against. This is the
 * byte-for-byte contract with `canonical_signing_message` in the Rust node (tx.rs), pinned
 * by the `canonical_sensor_commit_format_is_locked` test.
 *
 * CRITICAL: field order is INSERTION order, not alphabetical (the node's serde_json is
 * built with preserve_order). A client that sorted keys would produce signatures that fail
 * verification with no error pointing at the cause. Do not reorder these fields, and do not
 * route them through a JSON library that sorts keys — build the string directly.
 *
 * Verified byte-identical to the Rust output and to an independent Python/OpenSSL
 * implementation before this client was written.
 */
object CanonicalMessage {

    /** SENSOR_DATA_COMMIT — order: chain_id, type, sensor_id, owner, batch_hash, reading_count, sensor_type, signed_by */
    fun sensorDataCommit(
        chainId: String,
        sensorId: String,
        owner: String,
        batchHash: String,
        readingCount: Long,
        sensorType: String,
        signedBy: String,
    ): String = buildString {
        append('{')
        appendStr("chain_id", chainId); append(',')
        appendStr("type", "SENSOR_DATA_COMMIT"); append(',')
        appendStr("sensor_id", sensorId); append(',')
        appendStr("owner", owner); append(',')
        appendStr("batch_hash", batchHash); append(',')
        appendNum("reading_count", readingCount); append(',')
        appendStr("sensor_type", sensorType); append(',')
        appendStr("signed_by", signedBy)
        append('}')
    }

    /** SENSOR_REGISTER — order: chain_id, type, sensor_id, owner, sensor_type, location, signed_by */
    fun sensorRegister(
        chainId: String,
        sensorId: String,
        owner: String,
        sensorType: String,
        location: String?,
        signedBy: String,
    ): String = buildString {
        append('{')
        appendStr("chain_id", chainId); append(',')
        appendStr("type", "SENSOR_REGISTER"); append(',')
        appendStr("sensor_id", sensorId); append(',')
        appendStr("owner", owner); append(',')
        appendStr("sensor_type", sensorType); append(',')
        if (location == null) appendNull("location") else appendStr("location", location); append(',')
        appendStr("signed_by", signedBy)
        append('}')
    }

    /** TRANSFER — order: chain_id, type, from, to, amount, token, nonce (tx.rs). Signed with the active key. */
    fun transfer(chainId: String, from: String, to: String, amount: Long, token: String, nonce: Long): String = buildString {
        append('{')
        appendStr("chain_id", chainId); append(',')
        appendStr("type", "TRANSFER"); append(',')
        appendStr("from", from); append(',')
        appendStr("to", to); append(',')
        appendNum("amount", amount); append(',')
        appendStr("token", token); append(',')
        appendNum("nonce", nonce)
        append('}')
    }

    /** NODE_ROLE_STAKE / NODE_ROLE_UNSTAKE — order: chain_id, type, node, role, staker, amount, nonce. */
    fun nodeRoleStake(chainId: String, unstake: Boolean, node: String, role: String, staker: String,
                      amount: Long, nonce: Long): String = buildString {
        append('{')
        appendStr("chain_id", chainId); append(',')
        appendStr("type", if (unstake) "NODE_ROLE_UNSTAKE" else "NODE_ROLE_STAKE"); append(',')
        appendStr("node", node); append(',')
        appendStr("role", role); append(',')
        appendStr("staker", staker); append(',')
        appendNum("amount", amount); append(',')
        appendNum("nonce", nonce)
        append('}')
    }

    // serde_json string escaping: quotes, backslash, and control chars. Sensor ids/types
    // here are ASCII, but escape defensively so a location string can't break the format.
    private fun StringBuilder.appendStr(key: String, value: String) {
        append('"').append(key).append("\":\"")
        for (c in value) when (c) {
            '"' -> append("\\\"")
            '\\' -> append("\\\\")
            '\n' -> append("\\n")
            '\r' -> append("\\r")
            '\t' -> append("\\t")
            else -> if (c < ' ') append("\\u%04x".format(c.code)) else append(c)
        }
        append('"')
    }
    private fun StringBuilder.appendNum(key: String, value: Long) {
        append('"').append(key).append("\":").append(value)
    }
    private fun StringBuilder.appendNull(key: String) {
        append('"').append(key).append("\":null")
    }
}

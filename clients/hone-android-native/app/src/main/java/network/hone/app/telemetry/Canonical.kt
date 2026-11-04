package network.hone.app.telemetry

import org.bouncycastle.crypto.params.Ed25519PrivateKeyParameters
import org.bouncycastle.crypto.params.Ed25519PublicKeyParameters
import org.bouncycastle.crypto.signers.Ed25519Signer

/**
 * Canonical JSON for signed device reports: object keys sorted, no whitespace, integers,
 * booleans, strings and null only (no floats, so every language prints the same bytes).
 * Matches Python json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False).
 * Pure Kotlin (no org.json) so it is unit-testable on the JVM.
 */
object Canonical {
    fun encode(v: Any?): String = StringBuilder().also { write(it, v) }.toString()

    private fun write(sb: StringBuilder, v: Any?) {
        when (v) {
            null -> sb.append("null")
            is Boolean -> sb.append(if (v) "true" else "false")
            is Int, is Long, is Short, is Byte -> sb.append(v.toString())
            is Double, is Float -> {
                val d = (v as Number).toDouble()
                require(d == Math.rint(d) && !d.isInfinite()) { "canonical JSON has no floats: $v" }
                sb.append(d.toLong().toString())
            }
            is String -> str(sb, v)
            is Map<*, *> -> {
                sb.append('{')
                v.entries.map { it.key.toString() to it.value }.sortedBy { it.first }.forEachIndexed { i, (k, x) ->
                    if (i > 0) sb.append(',')
                    str(sb, k); sb.append(':'); write(sb, x)
                }
                sb.append('}')
            }
            is Iterable<*> -> {
                sb.append('[')
                v.forEachIndexed { i, x -> if (i > 0) sb.append(','); write(sb, x) }
                sb.append(']')
            }
            else -> str(sb, v.toString())
        }
    }

    private fun str(sb: StringBuilder, s: String) {
        sb.append('"')
        for (c in s) when (c) {
            '"' -> sb.append("\\\"")
            '\\' -> sb.append("\\\\")
            '\n' -> sb.append("\\n")
            '\r' -> sb.append("\\r")
            '\t' -> sb.append("\\t")
            '\b' -> sb.append("\\b")
            '\u000c' -> sb.append("\\f")
            else -> if (c < ' ') sb.append("\\u%04x".format(c.code)) else sb.append(c)
        }
        sb.append('"')
    }

    fun hex(b: ByteArray) = b.joinToString("") { "%02x".format(it) }
    fun unhex(s: String): ByteArray = s.chunked(2).map { it.toInt(16).toByte() }.toByteArray()

    fun pubOf(seed: ByteArray): ByteArray = Ed25519PrivateKeyParameters(seed, 0).generatePublicKey().encoded

    fun sign(seed: ByteArray, msg: String): ByteArray {
        val b = msg.toByteArray(Charsets.UTF_8)
        return Ed25519Signer().run { init(true, Ed25519PrivateKeyParameters(seed, 0)); update(b, 0, b.size); generateSignature() }
    }

    fun verify(pub: ByteArray, msg: String, sig: ByteArray): Boolean = runCatching {
        val b = msg.toByteArray(Charsets.UTF_8)
        Ed25519Signer().run { init(false, Ed25519PublicKeyParameters(pub, 0)); update(b, 0, b.size); verifySignature(sig) }
    }.getOrDefault(false)

    /** Envelope sent to the indexer: the exact signed string plus key type, pubkey and signature. */
    fun envelope(payload: String, keyType: String, seed: ByteArray): Map<String, Any> =
        mapOf("payload" to payload, "key" to keyType, "pub" to hex(pubOf(seed)), "sig" to hex(sign(seed, payload)))
}

package network.hone.app.data

import java.net.HttpURLConnection
import java.net.URL
import java.security.PrivateKey
import java.security.Signature

/** Signs canonical HONE messages without exposing private key bytes. */
fun interface SensorSigner {
    fun sign(message: ByteArray): ByteArray
}

class Ed25519SensorSigner(private val privateKey: PrivateKey) : SensorSigner {
    override fun sign(message: ByteArray): ByteArray = Signature.getInstance("Ed25519").run {
        initSign(privateKey)
        update(message)
        sign()
    }
}

fun interface SensorTransport {
    fun post(path: String, jsonBody: String): SensorHttpResponse
}

data class SensorHttpResponse(val status: Int, val body: String) {
    val accepted: Boolean get() = status in 200..299 && body.contains("\"accepted\":true")
}

class NodeSensorTransport(nodeUrl: String) : SensorTransport {
    private val baseUrl = nodeUrl.trimEnd('/')

    override fun post(path: String, jsonBody: String): SensorHttpResponse {
        val connection = URL(baseUrl + path).openConnection() as HttpURLConnection
        return try {
            connection.requestMethod = "POST"
            connection.connectTimeout = 10_000
            connection.readTimeout = 15_000
            connection.doOutput = true
            connection.setRequestProperty("Content-Type", "application/json")
            connection.setRequestProperty("Accept", "application/json")
            connection.outputStream.use { it.write(jsonBody.toByteArray(Charsets.UTF_8)) }
            val status = connection.responseCode
            val stream = if (status in 200..299) connection.inputStream else connection.errorStream
            SensorHttpResponse(status, stream?.bufferedReader()?.use { it.readText() }.orEmpty())
        } finally {
            connection.disconnect()
        }
    }
}

data class SensorReadingBatch(
    val sensorId: String,
    val owner: String,
    val batchHash: String,
    val readingCount: Long,
    val sensorType: String,
    val value: Double? = null,
)

/**
 * HONE sensor HTTP client. The caller selects either the enrolled device signer
 * or the owner's posting signer. Private keys remain inside the supplied signer.
 */
class SensorCommitClient(
    private val chainId: String,
    private val transport: SensorTransport,
) {
    /** Sync before submit (DeviceSync.kt); null = commits without seq, as before. */
    private var sync: DeviceSyncEngine? = null

    constructor(chainId: String, transport: SensorTransport, sync: DeviceSyncEngine?) : this(chainId, transport) {
        this.sync = sync
    }

    fun enrollDevice(
        sensorId: String,
        owner: String,
        devicePublicKeyHex: String,
        ownerPostingSigner: SensorSigner,
    ): SensorHttpResponse {
        val canonical = SensorWire.sensorKeyRegister(chainId, sensorId, owner, devicePublicKeyHex)
        val signature = ownerPostingSigner.sign(canonical.toByteArray()).toHex()
        return transport.post("/api/sensor/key-register", SensorWire.enrollmentBody(
            sensorId, owner, devicePublicKeyHex, signature,
        ))
    }

    fun revokeDevice(
        sensorId: String,
        owner: String,
        ownerPostingSigner: SensorSigner,
    ): SensorHttpResponse = enrollDevice(sensorId, owner, "", ownerPostingSigner)

    fun postReading(batch: SensorReadingBatch, signer: SensorSigner): SensorHttpResponse {
        val sync = this.sync
        if (sync == null) {
            val canonical = SensorWire.sensorDataCommit(chainId, batch)
            val signature = signer.sign(canonical.toByteArray()).toHex()
            return transport.post("/api/sensor/commit", SensorWire.commitBody(batch, signature))
        }
        var last: SensorHttpResponse? = null
        val out = sync.submit(batch.sensorId) { seq, prev ->
            val canonical = SensorWire.sensorDataCommit(chainId, batch, seq, prev)
            val signature = signer.sign(canonical.toByteArray()).toHex()
            val r = runCatching { transport.post("/api/sensor/commit", SensorWire.commitBody(batch, signature, seq, prev)) }
                .getOrElse { SensorHttpResponse(0, it.message ?: "error") }
            last = r
            DeviceSyncEngine.Post(r.status, r.body)
        }
        return when (out) {
            DeviceSyncEngine.Outcome.Paused -> SensorHttpResponse(0, "paused: this device's history doesn't match the chain")
            else -> last ?: SensorHttpResponse(0, "offline")
        }
    }
}

object SensorWire {
    /** [seq]/[prevHead]: sync before submit, appended only when present (hone-node tx.rs with_device_log). */
    fun sensorDataCommit(chainId: String, batch: SensorReadingBatch, seq: Long? = null, prevHead: String? = null): String =
        DeviceSyncEngine.withDeviceLog(sensorDataCommitBase(chainId, batch), seq, prevHead)

    private fun sensorDataCommitBase(chainId: String, batch: SensorReadingBatch): String =
        "{\"chain_id\":${q(chainId)},\"type\":\"SENSOR_DATA_COMMIT\",\"sensor_id\":${q(batch.sensorId)}," +
            "\"owner\":${q(batch.owner)},\"batch_hash\":${q(batch.batchHash)}," +
            "\"reading_count\":${batch.readingCount},\"sensor_type\":${q(batch.sensorType)}," +
            "\"signed_by\":${q(batch.owner)}}"

    fun sensorKeyRegister(chainId: String, sensorId: String, owner: String, pubkey: String): String =
        "{\"chain_id\":${q(chainId)},\"type\":\"SENSOR_KEY_REGISTER\",\"sensor_id\":${q(sensorId)}," +
            "\"owner\":${q(owner)},\"device_pubkey\":${q(pubkey)},\"signed_by\":${q(owner)}}"

    fun commitBody(batch: SensorReadingBatch, signature: String, seq: Long? = null, prevHead: String? = null): String {
        val value = batch.value?.let { ",\"value\":$it" }.orEmpty()
        val log = seq?.let { ",\"seq\":$it" + (prevHead?.let { p -> ",\"prev_head\":${q(p)}" } ?: "") }.orEmpty()
        return "{\"sensor_id\":${q(batch.sensorId)},\"owner\":${q(batch.owner)}," +
            "\"batch_hash\":${q(batch.batchHash)},\"reading_count\":${batch.readingCount}," +
            "\"sensor_type\":${q(batch.sensorType)}$value$log,\"signature\":${q(signature)}}"
    }

    fun enrollmentBody(sensorId: String, owner: String, pubkey: String, signature: String): String =
        "{\"sensor_id\":${q(sensorId)},\"owner\":${q(owner)},\"device_pubkey\":${q(pubkey)}," +
            "\"signature\":${q(signature)}}"

    private fun q(value: String): String = buildString {
        append('"')
        value.forEach { c -> when (c) {
            '"' -> append("\\\""); '\\' -> append("\\\\"); '\n' -> append("\\n")
            '\r' -> append("\\r"); '\t' -> append("\\t")
            else -> if (c.code < 0x20) append("\\u%04x".format(c.code)) else append(c)
        } }
        append('"')
    }
}

private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it.toInt() and 0xff) }

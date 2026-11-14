package network.hone.app.fw

import org.bouncycastle.crypto.params.Ed25519PublicKeyParameters
import org.bouncycastle.crypto.signers.Ed25519Signer
import org.json.JSONObject
import java.security.MessageDigest
import java.util.Base64

/**
 * A HONE firmware release as published at https://honemesh.net/app/firmware/manifest.json.
 *
 * Envelope (format "hone-fw-manifest@1"):
 *   { "format": "hone-fw-manifest@1", "key_id": "<16 hex>", "body": "<JSON text>", "sig": "<base64 ed25519>" }
 * The signature covers the UTF-8 bytes of "hone-fw-manifest@1\n" + body, exactly as published, so no JSON
 * canonicalisation is needed. body:
 *   { "version": "1.4.0", "channel": "hone" | "official", "target": 7, "min_app": 29,
 *     "files": [ { "name": "update.fuf", "size": 123, "sha256": "<64 hex>", "url": "f7-1.4.0/update.fuf" }, ... ] }
 * The package is the standard Flipper update folder (update.fuf + firmware.dfu + resources, ...); it is copied
 * to /ext/update/<version>/ and the Flipper's own updater checks and flashes it from the SD card.
 *
 * The private release key stays on grouchly. The app only pins public keys and refuses everything else.
 */
object FirmwarePackage {
    const val FORMAT = "hone-fw-manifest@1"
    const val MANIFEST_URL = "https://honemesh.net/app/firmware/manifest.json"

    /**
     * Pinned release public keys (32-byte ed25519, hex). Empty = this build cannot install firmware
     * (fail closed). Set from the firmware agent's published key; never the private key.
     */
    val RELEASE_KEYS: List<String> = listOf()

    data class FwFile(val name: String, val size: Long, val sha256: String, val url: String)
    data class Release(val version: String, val channel: String, val target: Int, val minApp: Int, val files: List<FwFile>) {
        val manifest: FwFile? get() = files.firstOrNull { it.name == "update.fuf" }
        val totalBytes: Long get() = files.sumOf { it.size }
    }

    class Refused(msg: String) : Exception(msg)

    fun keyId(pub: ByteArray): String = sha256Hex(pub).take(16)

    /** Verifies the envelope against [keys] and parses the body. Throws [Refused] on anything wrong. */
    fun verify(envelope: String, keys: List<String> = RELEASE_KEYS, appVersionCode: Int = Int.MAX_VALUE): Release {
        if (keys.isEmpty()) throw Refused("no release key is pinned in this app build")
        val e = runCatching { JSONObject(envelope) }.getOrElse { throw Refused("manifest is not JSON") }
        if (e.optString("format") != FORMAT) throw Refused("unknown manifest format")
        val body = e.optString("body").ifEmpty { throw Refused("manifest has no body") }
        val sig = runCatching { Base64.getDecoder().decode(e.optString("sig")) }.getOrNull()
        if (sig == null || sig.size != 64) throw Refused("manifest is not signed")
        val msg = (FORMAT + "\n" + body).toByteArray(Charsets.UTF_8)
        val ok = keys.any { k ->
            runCatching {
                val pub = hex(k); require(pub.size == 32)
                Ed25519Signer().run { init(false, Ed25519PublicKeyParameters(pub, 0)); update(msg, 0, msg.size); verifySignature(sig) }
            }.getOrDefault(false)
        }
        if (!ok) throw Refused("manifest signature does not match the HONE release key")
        val b = JSONObject(body)
        val fa = b.optJSONArray("files") ?: throw Refused("manifest lists no files")
        val files = (0 until fa.length()).map { fa.getJSONObject(it) }.map {
            FwFile(it.getString("name"), it.getLong("size"), it.getString("sha256").lowercase(), it.getString("url"))
        }
        for (f in files) {
            if (!Regex("[A-Za-z0-9._-]{1,64}").matches(f.name) || f.name.startsWith(".")) throw Refused("bad file name ${f.name}")
            if (!Regex("[0-9a-f]{64}").matches(f.sha256)) throw Refused("bad hash for ${f.name}")
            if (f.size <= 0) throw Refused("bad size for ${f.name}")
        }
        val version = b.optString("version")
        if (!Regex("[0-9A-Za-z._-]{1,32}").matches(version)) throw Refused("bad version")
        val r = Release(version, b.optString("channel", "hone"), b.optInt("target", 7), b.optInt("min_app", 0), files)
        if (r.manifest == null) throw Refused("package has no update.fuf")
        if (r.target != 7) throw Refused("package is not for Flipper Zero (f7)")
        if (r.minApp > appVersionCode) throw Refused("this firmware needs a newer HONE app")
        return r
    }

    /** True when the bytes match the signed size and sha256. */
    fun matches(f: FwFile, data: ByteArray) = data.size.toLong() == f.size && sha256Hex(data) == f.sha256

    fun sha256Hex(b: ByteArray): String = MessageDigest.getInstance("SHA-256").digest(b).joinToString("") { "%02x".format(it) }
    fun md5Hex(b: ByteArray): String = MessageDigest.getInstance("MD5").digest(b).joinToString("") { "%02x".format(it) }
    fun hex(s: String): ByteArray = s.trim().chunked(2).map { it.toInt(16).toByte() }.toByteArray()

    /** Battery / charging gate (PowerInfo keys differ between firmware versions). */
    fun batteryOk(power: Map<String, String>): Pair<Boolean, Int?> {
        val level = (power["charge.level"] ?: power["charge_level"])?.toIntOrNull()
        val state = (power["charge.state"] ?: power["charge_state"] ?: "").lowercase()
        val charging = state.contains("charg") && !state.contains("discharg") || state == "charged"
        return ((level != null && level >= 30) || charging) to level
    }

    /** Firmware version from DeviceInfo keys. */
    fun versionOf(info: Map<String, String>): String? =
        info["firmware.version"] ?: info["firmware_version"] ?: info["firmware.branch"]

    /** Whether the running firmware is HONE's (our builds carry "hone" in the branch or version). */
    fun isHone(info: Map<String, String>): Boolean = listOf("firmware.branch", "firmware.version", "firmware_version", "firmware.origin.fork")
        .any { (info[it] ?: "").contains("hone", ignoreCase = true) }
}

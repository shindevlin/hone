package network.hone.app.fw

import org.bouncycastle.crypto.params.Ed25519PublicKeyParameters
import org.bouncycastle.crypto.signers.Ed25519Signer
import org.json.JSONObject
import java.security.MessageDigest

/**
 * A HONE firmware release in the firmware agent's release format (hone-2.2.0 onwards):
 *   manifest.json      { name, version, tag, commit, target: "f7", built, package,
 *                        install: { rpc_upload_dir, upload_from, start, recovery },
 *                        signature: { alg: "ed25519", file: "manifest.json.sig", pubkey_pem_sha256 },
 *                        files: [ { path, size, sha256 }, ... ] }
 *   manifest.json.sig  64-byte detached ed25519 signature over the exact bytes of manifest.json
 * File paths are relative to the manifest URL. Only files under install.upload_from (the standard Flipper
 * update folder: update.fuf, firmware.dfu, radio.bin, resources.ths, ...) go to the Flipper, into
 * install.rpc_upload_dir; install.start is the update.fuf the Flipper's updater runs.
 *
 * The private release key stays on grouchly (~/.config/hone-release). The app pins public keys only.
 */
object FirmwarePackage {
    const val MANIFEST_URL = "https://honemesh.net/app/firmware/manifest.json"

    /**
     * Pinned release public keys (raw 32-byte ed25519, hex). Empty = this build installs nothing.
     * hone release key: release-ed25519.pub.pem, sha256(pem) 74655e46b50257e3ff9e386a528dfcca20d030f4c0e36a036452aabd56dc9f6c.
     */
    val RELEASE_KEYS: List<String> = listOf("27ef54588674c3391e660056d873b89136e75125b5a433de0747191baa94a10d")

    data class FwFile(val name: String, val size: Long, val sha256: String, val url: String)
    data class Release(val version: String, val tag: String, val target: String, val minApp: Int,
                       val uploadDir: String, val start: String, val files: List<FwFile>) {
        /** Files that go to the Flipper; [FwFile.name] is the name inside [uploadDir]. */
        val manifest: FwFile? get() = files.firstOrNull { it.name == "update.fuf" }
        val totalBytes: Long get() = files.sumOf { it.size }
    }

    class Refused(msg: String) : Exception(msg)

    /** Ed25519 check of [sig] over [msg] against any pinned key. */
    fun signedBy(msg: ByteArray, sig: ByteArray, keys: List<String>): Boolean = sig.size == 64 && keys.any { k ->
        runCatching {
            val pub = hex(k); require(pub.size == 32)
            Ed25519Signer().run { init(false, Ed25519PublicKeyParameters(pub, 0)); update(msg, 0, msg.size); verifySignature(sig) }
        }.getOrDefault(false)
    }

    /** Verifies manifest.json against its detached signature and parses it. Throws [Refused] on anything wrong. */
    fun verify(manifest: ByteArray, sig: ByteArray, keys: List<String> = RELEASE_KEYS, appVersionCode: Int = Int.MAX_VALUE): Release {
        if (keys.isEmpty()) throw Refused("no release key is pinned in this app build")
        if (sig.size != 64) throw Refused("manifest is not signed")
        if (!signedBy(manifest, sig, keys)) throw Refused("manifest signature does not match the HONE release key")
        val m = runCatching { JSONObject(manifest.toString(Charsets.UTF_8)) }.getOrElse { throw Refused("manifest is not JSON") }
        if (m.optJSONObject("signature")?.optString("alg", "ed25519") != "ed25519") throw Refused("unsupported signature")
        if (m.optString("target") != "f7") throw Refused("package is not for Flipper Zero (f7)")
        val version = m.optString("version")
        if (!Regex("[0-9A-Za-z._-]{1,32}").matches(version)) throw Refused("bad version")
        val inst = m.optJSONObject("install") ?: throw Refused("manifest has no install section")
        val dir = inst.optString("rpc_upload_dir").trimEnd('/')
        val from = inst.optString("upload_from").let { if (it.isEmpty() || it.endsWith("/")) it else "$it/" }
        val start = inst.optString("start")
        if (!Regex("/ext/update/[A-Za-z0-9._-]{1,64}").matches(dir)) throw Refused("bad upload folder")
        if (start != "$dir/update.fuf") throw Refused("update must start from $dir/update.fuf")
        val fa = m.optJSONArray("files") ?: throw Refused("manifest lists no files")
        val files = (0 until fa.length()).map { fa.getJSONObject(it) }.mapNotNull {
            val path = it.getString("path")
            if (path.startsWith("/") || path.contains("..") || path.contains("\\") || !Regex("[A-Za-z0-9._/-]{1,128}").matches(path))
                throw Refused("bad file path $path")
            val sha = it.getString("sha256").lowercase(); val size = it.getLong("size")
            if (!Regex("[0-9a-f]{64}").matches(sha)) throw Refused("bad hash for $path")
            if (size <= 0) throw Refused("bad size for $path")
            if (!path.startsWith(from)) null                       // e.g. the .tgz bundle: not uploaded
            else path.removePrefix(from).also { n -> if (n.isEmpty() || n.contains("/")) throw Refused("bad file path $path") }
                .let { n -> FwFile(n, size, sha, path) }
        }
        val r = Release(version, m.optString("tag", version), "f7", m.optInt("min_app", 0), dir, start, files)
        if (r.manifest == null) throw Refused("package has no update.fuf")
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

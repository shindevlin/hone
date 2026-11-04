package network.hone.app.sightings

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyProperties
import android.security.keystore.WrappedKeyEntry
import android.util.Base64
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import javax.crypto.Mac
import javax.crypto.SecretKey

/**
 * Find-my seeds held in this phone's hardware (standing grant, no hide key on the phone).
 *
 *  1. A non-exportable RSA-2048 PURPOSE_WRAP_KEY key is created in the Keystore (StrongBox
 *     when present, else TEE) with an attestation challenge. "Export hardware key" writes
 *     its public key + attestation chain for tools/hone_findgrant.py.
 *  2. The tool (holding the hide key) checks the attestation, signs FIND_GRANT with the
 *     hide key and wraps, per device, PRK = HKDF-Extract(salt="", id_seed_g) to this key
 *     (Android ImportWrappedKey format) as an HMAC-SHA256 key.
 *  3. The phone imports each PRK as a non-exportable HMAC key. share_k(period) =
 *     HKDF-Expand(PRK, "hone-share"||be32(p)) = HMAC(PRK, "hone-share"||be32(p)||0x01) is
 *     computed inside the hardware.
 *
 * Stays in hardware: the wrapping key, every id_seed/PRK. Never on the phone: hide key.
 * Reaches app RAM briefly (never stored or logged, zeroed after use): share_k for the month
 * being searched/advertised, and the per-window sk_w derived from it in software (X25519
 * and AES-GCM need the raw scalar/key). pk_w, lid and tag are public.
 */
object HwKeys {
    private const val TAG = "HoneSight"
    const val WRAP_ALIAS = "hone.find.wrap.v1"
    private const val SEED_PREFIX = "hone.find.seed."
    const val EXPORT_FILE = "hone-hwkey.json"
    const val BUNDLE_FILE = "hone-grant.json"

    private fun ks() = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    private fun meta(ctx: Context) = File(ctx.filesDir, "hone-node/find_grant.json")
    fun challenge(account: String, devicePub: String) = IdKeys.sha256("hone-find-v1|$account|$devicePub".toByteArray())
    fun seedAlias(device: String, g: Int) = SEED_PREFIX + device.take(32) + ".g" + g

    data class Info(val securityLevel: String, val strongBox: Boolean, val spki: ByteArray, val hwKey: String)

    /** Create (once) the attested wrapping key. Tries StrongBox first, then TEE. */
    @Synchronized
    fun ensureWrapKey(ctx: Context, account: String): Info {
        val ks = ks()
        if (!ks.containsAlias(WRAP_ALIAS)) {
            val strong = Build.VERSION.SDK_INT >= 28 && ctx.packageManager.hasSystemFeature("android.hardware.strongbox_keystore")
            fun gen(sb: Boolean) = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_RSA, "AndroidKeyStore").run {
                initialize(KeyGenParameterSpec.Builder(WRAP_ALIAS, KeyProperties.PURPOSE_WRAP_KEY)
                    .setKeySize(2048)
                    .setDigests(KeyProperties.DIGEST_SHA256)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_RSA_OAEP)
                    .setAttestationChallenge(challenge(account, RoleKeys.devicePub(ctx).hex()))
                    .apply { if (sb && Build.VERSION.SDK_INT >= 28) setIsStrongBoxBacked(true) }
                    .build())
                generateKeyPair()
            }
            if (strong) runCatching { gen(true) }.onFailure { Log.w(TAG, "StrongBox wrap key failed, using TEE"); gen(false) }
            else gen(false)
        }
        return info()
    }

    fun info(): Info {
        val ks = ks()
        val cert = ks.getCertificate(WRAP_ALIAS) ?: error("no hardware key yet")
        val priv = ks.getKey(WRAP_ALIAS, null) as java.security.PrivateKey
        val ki = KeyFactory.getInstance(priv.algorithm, "AndroidKeyStore").getKeySpec(priv, KeyInfo::class.java)
        val level = if (Build.VERSION.SDK_INT >= 31) when (ki.securityLevel) {
            KeyProperties.SECURITY_LEVEL_STRONGBOX -> "strongbox"
            KeyProperties.SECURITY_LEVEL_TRUSTED_ENVIRONMENT -> "tee"
            KeyProperties.SECURITY_LEVEL_SOFTWARE -> "software"
            else -> "unknown"
        } else if (@Suppress("DEPRECATION") ki.isInsideSecureHardware) "tee" else "software"
        val spki = cert.publicKey.encoded
        return Info(level, level == "strongbox", spki, IdKeys.sha256(spki).hex())
    }

    /** Write the wrapping public key + attestation chain to the app's external files dir. */
    fun export(ctx: Context, account: String): File {
        val i = ensureWrapKey(ctx, account)
        val chain = ks().getCertificateChain(WRAP_ALIAS).map { Base64.encodeToString(it.encoded, Base64.NO_WRAP) }
        val o = JSONObject().put("v", 1).put("account", account).put("device", RoleKeys.devicePub(ctx).hex())
            .put("package", ctx.packageName).put("security_level", i.securityLevel)
            .put("hw_key", i.hwKey).put("wrap_spki", Base64.encodeToString(i.spki, Base64.NO_WRAP))
            .put("chain", JSONArray(chain)).put("sdk", Build.VERSION.SDK_INT).put("model", Build.MODEL)
        val f = File(ctx.getExternalFilesDir(null), EXPORT_FILE); f.writeText(o.toString(2))
        return f
    }

    data class Granted(val device: String, val generation: Int, val label: String)

    /**
     * Import the bundle written by hone_findgrant.py. Each wrapped PRK becomes a
     * non-exportable HMAC key; the bundle file is deleted afterwards (it holds only
     * ciphertext, but there is no reason to keep it). Returns a status line.
     */
    @Synchronized
    fun importBundle(ctx: Context, file: File = File(ctx.getExternalFilesDir(null), BUNDLE_FILE)): String {
        if (!file.exists()) return "no ${file.name} in ${file.parent}"
        val b = JSONObject(file.readText())
        val me = info()
        require(b.getString("hw_key") == me.hwKey) { "bundle is for another phone's hardware key" }
        val ks = ks()
        // Keystore reads the unwrapping parameters (RSA-OAEP, SHA-256, MGF1-SHA-1 default) from a KeyGenParameterSpec.
        val spec = KeyGenParameterSpec.Builder(WRAP_ALIAS, KeyProperties.PURPOSE_WRAP_KEY)
            .setDigests(KeyProperties.DIGEST_SHA256)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_RSA_OAEP).build()
        val devs = b.getJSONArray("devices")
        val out = JSONArray()
        for (i in 0 until devs.length()) {
            val d = devs.getJSONObject(i)
            val dev = d.getString("device"); val g = d.optInt("generation", 0)
            val alias = seedAlias(dev, g)
            if (ks.containsAlias(alias)) ks.deleteEntry(alias)
            val wrapped = Base64.decode(d.getString("wrapped"), Base64.DEFAULT)
            ks.setEntry(alias, WrappedKeyEntry(wrapped, WRAP_ALIAS, "RSA/ECB/OAEPPadding", spec), null)
            out.put(JSONObject().put("device", dev).put("generation", g).put("label", d.optString("label")))
        }
        val m = JSONObject().put("owner", b.getString("owner")).put("hw_key", me.hwKey)
            .put("dry_run", b.optBoolean("dry_run", false)).put("grant", b.optJSONObject("grant"))
            .put("devices", out).put("imported_ms", System.currentTimeMillis())
        b.optJSONObject("selftest")?.let { m.put("selftest", it) }   // public test vectors only
        meta(ctx).apply { parentFile?.mkdirs() }.writeText(m.toString())
        file.delete()
        Log.i(TAG, "find grant imported: ${out.length()} device seed(s) into ${me.securityLevel}")
        return "imported ${out.length()} device seed(s) into ${me.securityLevel}" + if (m.optBoolean("dry_run")) " (TEST grant)" else ""
    }

    fun grant(ctx: Context): JSONObject? = runCatching { JSONObject(meta(ctx).readText()) }.getOrNull()

    fun granted(ctx: Context): List<Granted> {
        val a = grant(ctx)?.optJSONArray("devices") ?: return emptyList()
        return (0 until a.length()).map { a.getJSONObject(it) }.map { Granted(it.getString("device"), it.optInt("generation"), it.optString("label")) }
    }

    fun owner(ctx: Context): String? = grant(ctx)?.optString("owner")?.takeIf { it.isNotEmpty() }

    /** share_k for a granted device, computed by the hardware HMAC key. Caller zeroes it. */
    fun shareKey(device: String, generation: Int, period: Int): ByteArray? {
        val k = ks().getKey(seedAlias(device, generation), null) as? SecretKey ?: return null
        val m = Mac.getInstance("HmacSHA256"); m.init(k)
        m.update("hone-share".toByteArray(Charsets.US_ASCII))
        m.update(java.nio.ByteBuffer.allocate(4).putInt(period).array())
        m.update(byteArrayOf(1))
        return m.doFinal()
    }

    fun hasSeed(device: String, generation: Int) = ks().containsAlias(seedAlias(device, generation))

    /** Drop every imported seed and the grant record (the wrapping key stays). */
    fun forgetGrant(ctx: Context) {
        val ks = ks()
        ks.aliases().toList().filter { it.startsWith(SEED_PREFIX) }.forEach { ks.deleteEntry(it) }
        meta(ctx).delete()
    }

    /** Self-test against the tool's reference vectors: lids for some windows, and one sealed
     *  test sighting. Returns (matched, total, decrypted plaintext or null). */
    fun selfTest(ctx: Context): Triple<Int, Int, String?> {
        val t = grant(ctx)?.optJSONObject("selftest") ?: error("no self-test vectors in this grant")
        val dev = t.getString("device"); val g = t.optInt("generation")
        val vec = t.getJSONArray("lids")
        var ok = 0
        val cache = HashMap<Int, ByteArray>()
        try {
            for (i in 0 until vec.length()) {
                val v = vec.getJSONObject(i); val w = v.getLong("w")
                val sk = cache.getOrPut(IdKeys.period(w)) { shareKey(dev, g, IdKeys.period(w)) ?: error("seed not in hardware") }
                val wk = IdKeys.windowKey(sk, w)
                if (wk.lid.hex() == v.getString("lid")) ok++
                wk.sk.fill(0)
            }
            val s = t.optJSONObject("sighting")
            val pt = s?.let {
                val w = it.getLong("w")
                val sk = cache.getOrPut(IdKeys.period(w)) { shareKey(dev, g, IdKeys.period(w))!! }
                val wk = IdKeys.windowKey(sk, w)
                val r = IdKeys.open(wk.sk, wk.pk, SightingsEngine.INFO_SIGHTING, Base64.decode(it.getString("blob"), Base64.DEFAULT), aad = wk.lid)
                wk.sk.fill(0); r?.let { b -> String(b) }
            }
            return Triple(ok, vec.length(), pt)
        } finally { cache.values.forEach { it.fill(0) } }
    }
}
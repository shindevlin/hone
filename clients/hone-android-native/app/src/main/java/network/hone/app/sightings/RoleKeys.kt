package network.hone.app.sightings

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Log
import java.io.File
import java.security.KeyStore
import java.security.SecureRandom
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Owner wallet role keys this phone may hold, imported by the owner:
 *   hide   -> derives every owned device's id_seed (find my things, advert keys)
 *   memo   -> decrypts location-share grants addressed to this account
 *   active -> optional; only needed to sign grant/revoke (MemorySet) from the phone
 *
 * Each is stored wrapped by a non-exportable AndroidKeyStore AES-GCM key
 * (files/hone-node/role_<r>.wrapped). Never logged or exported: the wallet mnemonic is
 * the backup. Imports are checked against the account's on-chain role key when the chain
 * has one registered.
 *
 * Until a hide key is imported, this phone uses a provisional random seed (the old
 * id_seed file). It is replaced (one-time key change) by the derived seed on import.
 */
object RoleKeys {
    private const val TAG = "HoneSight"
    private const val ALIAS = "hone.rolekeys.wrap"
    val ROLES = listOf("hide", "memo", "active", "posting")
    private val cache = HashMap<String, ByteArray>()

    private fun file(ctx: Context, role: String) = File(ctx.filesDir, "hone-node/role_$role.wrapped")

    private fun wrapKey(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256).build())
            generateKey()
        }
    }

    @Synchronized
    fun get(ctx: Context, role: String): ByteArray? {
        cache[role]?.let { return it }
        val f = file(ctx, role)
        if (!f.exists()) return null
        return runCatching {
            val b = f.readBytes()
            val c = Cipher.getInstance("AES/GCM/NoPadding")
            c.init(Cipher.DECRYPT_MODE, wrapKey(), GCMParameterSpec(128, b.copyOfRange(0, 12)))
            c.doFinal(b, 12, b.size - 12)
        }.onFailure { Log.e(TAG, "role key $role unwrap failed") }.getOrNull()?.also { cache[role] = it }
    }

    fun has(ctx: Context, role: String) = file(ctx, role).exists()

    /**
     * Import a role private key (64 hex). [chainPub] is the account's on-chain public key
     * for that role, if registered; a mismatch is refused. Returns an error text or null.
     */
    @Synchronized
    fun import(ctx: Context, role: String, privHex: String, chainPub: String?): String? {
        require(role in ROLES)
        val h = privHex.trim().lowercase()
        if (!h.matches(Regex("[0-9a-f]{64}"))) return "expected 64 hex characters"
        val priv = h.unhex()
        val pub = IdKeys.ed25519Pub(priv).hex()
        if (!chainPub.isNullOrEmpty() && chainPub != pub) return "does not match the account's on-chain $role key"
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.ENCRYPT_MODE, wrapKey())
        val f = file(ctx, role); f.parentFile?.mkdirs()
        val tmp = File(f.parentFile, f.name + ".tmp")
        tmp.writeBytes(c.iv + c.doFinal(priv)); tmp.renameTo(f)
        cache[role] = priv
        Log.i(TAG, "role key $role imported (pub ${pub.take(8)}..)")
        return null
    }

    fun forget(ctx: Context, role: String) { synchronized(this) { cache.remove(role); file(ctx, role).delete() } }

    // ── this device's identity ──────────────────────────────────────────────────

    /** This phone's device pubkey: the witness seat key (josh@<pubkey>), created if absent
     *  in the same format the Rust core uses (hex secret, mode 600). */
    fun devicePub(ctx: Context): ByteArray {
        val f = File(ctx.filesDir, "hone-node/witness.key")
        if (!f.exists()) {
            f.parentFile?.mkdirs()
            val sk = ByteArray(32).also { SecureRandom().nextBytes(it) }
            f.writeText(sk.hex())
            f.setReadable(false, false); f.setReadable(true, true)
        }
        return IdKeys.ed25519Pub(f.readText().trim().unhex())
    }

    /** id_seed for a device of this owner: derived from hide when imported, else null. */
    fun seedFor(ctx: Context, devicePub: ByteArray): ByteArray? =
        get(ctx, "hide")?.let { IdKeys.deviceSeed(it, devicePub) }

    /**
     * This phone's window key and where its seed lives: "hardware" (standing grant, seed in
     * Keystore), "hide" (legacy hide key on the phone) or "provisional" (random, unlinked).
     * share_k is zeroed here; the returned sk is for the caller to zero when done.
     */
    fun thisWindowKey(ctx: Context, window: Long): Pair<IdKeys.WindowKey, String> {
        val me = devicePub(ctx).hex()
        HwKeys.granted(ctx).firstOrNull { it.device == me }?.let { g ->
            runCatching { HwKeys.shareKey(me, g.generation, IdKeys.period(window)) }.getOrNull()?.let { sk ->
                // Advertising needs only pk/lid/tag: sk_w and share_k are zeroed here.
                try { return IdKeys.windowKey(sk, window).also { it.sk.fill(0) } to "hardware" } finally { sk.fill(0) }
            }
        }
        val (s, d) = thisSeed(ctx)
        return IdKeys.windowKeyFromSeed(s, window) to (if (d) "hide" else "provisional")
    }

    @Volatile private var provisional: ByteArray? = null

    /** This phone's seed and whether it is derived (true) or provisional (false). */
    fun thisSeed(ctx: Context): Pair<ByteArray, Boolean> {
        seedFor(ctx, devicePub(ctx))?.let {
            File(ctx.filesDir, "hone-node/id_seed").delete()   // migration: the random seed is gone
            return it to true
        }
        provisional?.let { return it to false }
        val f = File(ctx.filesDir, "hone-node/id_seed")
        val s = if (f.exists() && f.length() == 32L) f.readBytes() else
            ByteArray(32).also { SecureRandom().nextBytes(it); f.parentFile?.mkdirs(); f.writeBytes(it) }
        provisional = s
        return s to false
    }
}

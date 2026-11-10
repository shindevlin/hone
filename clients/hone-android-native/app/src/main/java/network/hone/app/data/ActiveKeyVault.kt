package network.hone.app.data

import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.SecretKeyFactory
import kotlin.coroutines.resume

/**
 * The account's ACTIVE key (sends HONE, backs people, sets the agent key).
 *
 * Stored as VaultCrypto's two layers: Argon2id(password) + AES-GCM inside, an Android
 * Keystore AES-GCM key (hardware, StrongBox where the phone has it) outside. Only the
 * ciphertext touches disk (files/hone-node/wallet_active.v2, excluded from backup and
 * device transfer). The password is asked for EVERY signature, behind a confirm screen
 * showing the action, the amount in HONE and the destination ([VaultPrompt]). An optional
 * fingerprint is an extra lock on the Keystore key, never a replacement for the password.
 * The seed is decrypted for one signature and zeroed; nothing stays unlocked.
 * After 5 wrong passwords each try waits longer (30 s doubling, up to 1 h).
 */
object ActiveKeyVault {
    private const val ALIAS = "hone.wallet.active.v2"
    private const val LEGACY_ALIAS = "hone.wallet.active.v1"
    private fun dir(ctx: Context) = File(ctx.filesDir, "hone-node")
    private fun file(ctx: Context) = File(dir(ctx), "wallet_active.v2")
    private fun pubFile(ctx: Context) = File(dir(ctx), "wallet_active.pub")
    private fun bioFile(ctx: Context) = File(dir(ctx), "wallet_active.bio")
    private fun legacyFile(ctx: Context) = File(dir(ctx), "wallet_active.enc")
    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_vault", Context.MODE_PRIVATE)

    const val MIN_PASSWORD = 10

    fun has(ctx: Context) = file(ctx).exists() && pubFile(ctx).exists()
    fun pub(ctx: Context): String? = pubFile(ctx).takeIf { has(ctx) }?.readText()?.trim()
    fun usesBiometric(ctx: Context) = bioFile(ctx).exists()
    /** A key stored by app <= 1.13 (fingerprint only, no password). It can no longer sign. */
    fun hasLegacy(ctx: Context) = legacyFile(ctx).exists()

    /** Wipe the stored key, its Keystore keys and the retry counter. */
    fun remove(ctx: Context) {
        listOf(file(ctx), pubFile(ctx), bioFile(ctx), legacyFile(ctx), File(dir(ctx), "wallet_active.v2.tmp")).forEach { f ->
            if (f.exists()) { runCatching { f.writeBytes(ByteArray(f.length().toInt())) }; f.delete() }
        }
        runCatching { KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.apply { deleteEntry(ALIAS); deleteEntry(LEGACY_ALIAS) } }
        prefs(ctx).edit().clear().apply()
    }

    /** "StrongBox", "hardware (TEE)" or "software" for the stored Keystore key. */
    fun protection(): String = runCatching {
        val key = existingKey() ?: return "none"
        val info = SecretKeyFactory.getInstance(key.algorithm, "AndroidKeyStore").getKeySpec(key, KeyInfo::class.java) as KeyInfo
        when {
            Build.VERSION.SDK_INT >= 31 && info.securityLevel == KeyProperties.SECURITY_LEVEL_STRONGBOX -> "StrongBox"
            Build.VERSION.SDK_INT >= 31 && info.securityLevel == KeyProperties.SECURITY_LEVEL_TRUSTED_ENVIRONMENT -> "hardware (TEE)"
            @Suppress("DEPRECATION") info.isInsideSecureHardware -> "hardware"
            else -> "software"
        }
    }.getOrDefault("unknown")

    private fun newKey(ctx: Context, biometric: Boolean): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        if (ks.containsAlias(ALIAS)) ks.deleteEntry(ALIAS)
        fun spec(strongBox: Boolean) = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .apply {
                if (Build.VERSION.SDK_INT >= 28) { setIsStrongBoxBacked(strongBox); setUnlockedDeviceRequired(true) }
                if (biometric) {
                    setUserAuthenticationRequired(true)
                    setInvalidatedByBiometricEnrollment(true)
                    if (Build.VERSION.SDK_INT >= 30) setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
                    else @Suppress("DEPRECATION") setUserAuthenticationValidityDurationSeconds(-1)
                }
            }.build()
        val strong = Build.VERSION.SDK_INT >= 28 && ctx.packageManager.hasSystemFeature(PackageManager.FEATURE_STRONGBOX_KEYSTORE)
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        return try { gen.init(spec(strong)); gen.generateKey() }
        catch (e: Exception) {
            // StrongBoxUnavailableException (or a provider error from the StrongBox HAL): use the TEE.
            if (!strong) throw e
            if (ks.containsAlias(ALIAS)) ks.deleteEntry(ALIAS)
            gen.init(spec(false)); gen.generateKey()
        }
    }

    private fun existingKey(): SecretKey? =
        KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.getKey(ALIAS, null) as? SecretKey

    @Deprecated("use VaultCrypto.pubOf", ReplaceWith("VaultCrypto.pubOf(seed)"))
    fun pubOf(seed: ByteArray): String = VaultCrypto.pubOf(seed)

    /**
     * Import [privHex] (64 hex Ed25519 seed) for an account whose on-chain active key is
     * [chainActivePub], locked with [password] (zeroed here) and optionally a fingerprint.
     * Returns an error, or null on success.
     */
    suspend fun import(activity: FragmentActivity, privHex: String, password: CharArray, chainActivePub: String?, biometric: Boolean): String? {
        var seed: ByteArray? = null
        try {
            if (password.size < MIN_PASSWORD) return "Use a password of at least $MIN_PASSWORD characters"
            seed = runCatching { VaultCrypto.parseSeedHex(privHex) }.getOrElse { return it.message }
            VaultCrypto.checkAgainstChain(seed, chainActivePub)?.let { return it }
            val s = seed
            val inner = withContext(Dispatchers.Default) { VaultCrypto.seal(s, password) }
            val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, newKey(activity, biometric)) }
            val ready = if (biometric) authenticate(activity, cipher, "Add fingerprint lock", "Your password is still needed for every use")
                ?: run { remove(activity); return "Fingerprint cancelled; not imported" } else cipher
            val blob = VaultCrypto.wrap(ready, inner)
            withContext(Dispatchers.IO) {
                val d = dir(activity); d.mkdirs()
                val tmp = File(d, "wallet_active.v2.tmp"); tmp.writeBytes(blob)
                if (!tmp.renameTo(file(activity))) error("could not write the vault")
                pubFile(activity).writeText(chainActivePub!!.trim().lowercase())
                if (biometric) bioFile(activity).writeText("1") else bioFile(activity).delete()
                if (legacyFile(activity).exists()) { legacyFile(activity).delete()
                    runCatching { KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.deleteEntry(LEGACY_ALIAS) } }
            }
            prefs(activity).edit().clear().apply()
            return null
        } catch (e: Exception) {
            return "Could not store the key (${e.javaClass.simpleName})"
        } finally { seed?.fill(0); password.fill('\u0000') }
    }

    sealed class Signed {
        data class Ok(val signatureHex: String, val pub: String) : Signed()
        data class Failed(val reason: String) : Signed()
    }

    /** Milliseconds until the next password try is allowed (0 = now). */
    fun waitMs(ctx: Context): Long {
        val p = prefs(ctx)
        val until = p.getLong("until", 0L)
        val now = System.currentTimeMillis()
        val d = until - now
        val max = VaultBackoff.delayMs(p.getInt("fails", 0))
        return if (d <= 0) 0 else minOf(d, maxOf(max, 1_000L))   // clock moved back: never wait longer than the rule
    }
    private fun failed(ctx: Context) {
        val p = prefs(ctx); val n = p.getInt("fails", 0) + 1
        p.edit().putInt("fails", n).putLong("until", System.currentTimeMillis() + VaultBackoff.delayMs(n)).apply()
    }
    fun failures(ctx: Context) = prefs(ctx).getInt("fails", 0)

    /**
     * Show the confirm screen for [confirm], ask for the password (and fingerprint, if set),
     * decrypt, sign [message] (UTF-8) once and wipe the key. No signature without this screen.
     */
    suspend fun sign(activity: FragmentActivity, message: String, confirm: VaultPrompt.Confirm): Signed {
        if (!has(activity)) return Signed.Failed(if (hasLegacy(activity))
            "The sending key on this phone uses the old fingerprint-only lock. Remove it and import it again with a password." else "No active key on this phone")
        var error: String? = null
        while (true) {
            val password = VaultPrompt.ask(confirm.copy(error = error, waitUntil = System.currentTimeMillis() + waitMs(activity)))
                ?: return Signed.Failed("Cancelled; nothing was signed")
            try {
                val wait = waitMs(activity)
                if (wait > 0) { error = "Too many wrong passwords. Try again in ${(wait + 999) / 1000} s."; continue }
                val outer = withContext(Dispatchers.IO) { file(activity).readBytes() }
                val key = existingKey() ?: return Signed.Failed("Active key protection is missing; remove and import the key again")
                val cipher = try {
                    Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE, key, VaultCrypto.outerIv(outer)) }
                } catch (e: KeyPermanentlyInvalidatedException) {
                    return Signed.Failed("Your fingerprints changed, which retired the stored key. Remove it and import again.")
                }
                val ready = if (usesBiometric(activity)) authenticate(activity, cipher, confirm.action, "Fingerprint, then your password is checked")
                    ?: return Signed.Failed("Fingerprint cancelled; nothing was signed") else cipher
                val inner = VaultCrypto.unwrap(ready, outer)
                val (sig, pub) = withContext(Dispatchers.Default) {
                    VaultCrypto.signOnce(inner, password, message.toByteArray(Charsets.UTF_8))
                }
                prefs(activity).edit().clear().apply()
                return Signed.Ok(sig, pub)
            } catch (e: VaultCrypto.WrongPassword) {
                failed(activity)
                val left = VaultBackoff.FREE - failures(activity)
                error = if (left > 0) "Wrong password ($left more before a wait)" else "Wrong password. Wait ${(waitMs(activity) + 999) / 1000} s."
            } catch (e: VaultCrypto.Tampered) {
                return Signed.Failed("The stored key is damaged or was changed. Remove it and import it again.")
            } catch (e: Exception) {
                return Signed.Failed("Signing failed (${e.javaClass.simpleName})")
            } finally { password.fill('\u0000') }
        }
    }

    /** BiometricPrompt bound to [cipher]; returns the authorised cipher, or null if cancelled/failed. */
    private suspend fun authenticate(activity: FragmentActivity, cipher: Cipher, title: String, subtitle: String): Cipher? =
        withContext(Dispatchers.Main) {
            suspendCancellableCoroutine { cont ->
                val prompt = BiometricPrompt(activity, ContextCompat.getMainExecutor(activity),
                    object : BiometricPrompt.AuthenticationCallback() {
                        override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                            if (cont.isActive) cont.resume(result.cryptoObject?.cipher)
                        }
                        override fun onAuthenticationError(code: Int, msg: CharSequence) {
                            if (cont.isActive) cont.resume(null)
                        }
                    })
                val info = BiometricPrompt.PromptInfo.Builder().setTitle(title).setSubtitle(subtitle)
                    .setAllowedAuthenticators(BIOMETRIC_STRONG).setNegativeButtonText("Cancel").build()
                prompt.authenticate(info, BiometricPrompt.CryptoObject(cipher))
                cont.invokeOnCancellation { runCatching { prompt.cancelAuthentication() } }
            }
        }
}

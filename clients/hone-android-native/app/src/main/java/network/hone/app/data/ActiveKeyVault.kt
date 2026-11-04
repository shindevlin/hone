package network.hone.app.data

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricManager.Authenticators.DEVICE_CREDENTIAL
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import org.bouncycastle.crypto.params.Ed25519PrivateKeyParameters
import org.bouncycastle.crypto.signers.Ed25519Signer
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import kotlin.coroutines.resume

/**
 * The account's ACTIVE key, pasted in by its owner, for moving tokens (Transfer,
 * NodeRoleStake/Unstake). Kept apart from the posting key used for birds (RoleKeys,
 * no biometric).
 *
 * Stored as AES-GCM ciphertext under an AndroidKeyStore key that requires the user's
 * biometric or device credential for EVERY use (auth timeout 0, CryptoObject-bound), so
 * each signature needs its own prompt. Never logged, never shown again, never sent:
 * only signatures leave the phone. Imports are refused unless the derived public key
 * equals the account's on-chain active key.
 */
object ActiveKeyVault {
    private const val ALIAS = "hone.wallet.active.v1"
    private fun file(ctx: Context) = File(ctx.filesDir, "hone-node/wallet_active.enc")
    private fun pubFile(ctx: Context) = File(ctx.filesDir, "hone-node/wallet_active.pub")

    fun has(ctx: Context) = file(ctx).exists() && pubFile(ctx).exists()
    fun pub(ctx: Context): String? = pubFile(ctx).takeIf { it.exists() }?.readText()?.trim()

    fun remove(ctx: Context) {
        file(ctx).delete(); pubFile(ctx).delete()
        runCatching { KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.deleteEntry(ALIAS) }
    }

    private fun newKey(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        if (ks.containsAlias(ALIAS)) ks.deleteEntry(ALIAS)
        val spec = KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setUserAuthenticationRequired(true)
            .setInvalidatedByBiometricEnrollment(true)
            .apply {
                if (Build.VERSION.SDK_INT >= 30)
                    setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG or KeyProperties.AUTH_DEVICE_CREDENTIAL)
                else @Suppress("DEPRECATION") setUserAuthenticationValidityDurationSeconds(-1)
            }
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run { init(spec); generateKey() }
    }

    private fun existingKey(): SecretKey? =
        KeyStore.getInstance("AndroidKeyStore").apply { load(null) }.getKey(ALIAS, null) as? SecretKey

    fun pubOf(seed: ByteArray): String =
        Ed25519PrivateKeyParameters(seed, 0).generatePublicKey().encoded.joinToString("") { "%02x".format(it) }

    /**
     * Import [privHex] (64 hex, the Ed25519 seed). [chainActivePub] is the account's
     * on-chain active key; null/empty or a mismatch is refused. Returns an error or null.
     */
    suspend fun import(activity: FragmentActivity, privHex: String, chainActivePub: String?): String? {
        val h = privHex.trim().lowercase()
        if (!h.matches(Regex("[0-9a-f]{64}"))) return "Expected 64 hex characters (0-9, a-f); got ${h.length}"
        if (chainActivePub.isNullOrEmpty()) return "Could not read the account's active key from the chain; not imported"
        val seed = ByteArray(32) { i -> h.substring(i * 2, i * 2 + 2).toInt(16).toByte() }
        try {
            if (pubOf(seed) != chainActivePub.lowercase())
                return "This is not the account's active key (public key does not match the chain); not imported"
            val cipher = Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.ENCRYPT_MODE, newKey()) }
            val c = authenticate(activity, cipher, "Store active key", "Protect the active key with your fingerprint or screen lock")
                ?: return "Authentication cancelled; not imported"
            val blob = c.iv + c.doFinal(seed)
            withContext(Dispatchers.IO) {
                val f = file(activity); f.parentFile?.mkdirs()
                val tmp = File(f.parentFile, f.name + ".tmp"); tmp.writeBytes(blob); tmp.renameTo(f)
                pubFile(activity).writeText(chainActivePub.lowercase())
            }
            return null
        } catch (e: Exception) {
            return "Could not store the key (${e.javaClass.simpleName})"
        } finally { seed.fill(0) }
    }

    sealed class Signed {
        data class Ok(val signatureHex: String, val pub: String) : Signed()
        data class Failed(val reason: String) : Signed()
    }

    /** Ask for biometric/credential, decrypt the active key, sign [message] (UTF-8), wipe the key. */
    suspend fun sign(activity: FragmentActivity, message: String, title: String, subtitle: String): Signed {
        val f = file(activity)
        if (!f.exists()) return Signed.Failed("No active key on this phone")
        val blob = withContext(Dispatchers.IO) { f.readBytes() }
        val key = existingKey() ?: return Signed.Failed("Active key protection is missing; remove and import the key again")
        val cipher = try {
            Cipher.getInstance("AES/GCM/NoPadding").apply { init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(128, blob, 0, 12)) }
        } catch (e: KeyPermanentlyInvalidatedException) {
            return Signed.Failed("Your fingerprints or screen lock changed, which retired the stored key. Remove it and import again.")
        } catch (e: Exception) {
            return Signed.Failed("Could not open the stored key (${e.javaClass.simpleName})")
        }
        val c = authenticate(activity, cipher, title, subtitle) ?: return Signed.Failed("Authentication cancelled; nothing was signed")
        var seed: ByteArray? = null
        return try {
            seed = c.doFinal(blob, 12, blob.size - 12)
            val sk = Ed25519PrivateKeyParameters(seed, 0)
            val pub = sk.generatePublicKey().encoded.joinToString("") { "%02x".format(it) }
            val msg = message.toByteArray(Charsets.UTF_8)
            val sig = Ed25519Signer().run { init(true, sk); update(msg, 0, msg.size); generateSignature() }
            Signed.Ok(sig.joinToString("") { "%02x".format(it) }, pub)
        } catch (e: Exception) {
            Signed.Failed("Signing failed (${e.javaClass.simpleName})")
        } finally { seed?.fill(0) }
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
                val info = BiometricPrompt.PromptInfo.Builder().setTitle(title).setSubtitle(subtitle).apply {
                    if (Build.VERSION.SDK_INT >= 30) setAllowedAuthenticators(BIOMETRIC_STRONG or DEVICE_CREDENTIAL)
                    else { setAllowedAuthenticators(BIOMETRIC_STRONG); setNegativeButtonText("Cancel") }
                }.build()
                prompt.authenticate(info, BiometricPrompt.CryptoObject(cipher))
                cont.invokeOnCancellation { runCatching { prompt.cancelAuthentication() } }
            }
        }
}
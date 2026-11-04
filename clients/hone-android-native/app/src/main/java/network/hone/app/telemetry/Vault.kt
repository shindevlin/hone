package network.hone.app.telemetry

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** AES-GCM sealing with non-exportable AndroidKeyStore keys (one per alias). Output: iv(12) + ciphertext. */
object Vault {
    private fun key(alias: String): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(alias, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256).build())
            generateKey()
        }
    }

    fun seal(alias: String, plain: ByteArray): ByteArray {
        val c = Cipher.getInstance("AES/GCM/NoPadding"); c.init(Cipher.ENCRYPT_MODE, key(alias))
        return c.iv + c.doFinal(plain)
    }

    fun open(alias: String, blob: ByteArray): ByteArray? = runCatching {
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.DECRYPT_MODE, key(alias), GCMParameterSpec(128, blob.copyOfRange(0, 12)))
        c.doFinal(blob, 12, blob.size - 12)
    }.getOrNull()
}

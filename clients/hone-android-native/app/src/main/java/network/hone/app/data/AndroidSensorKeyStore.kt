package network.hone.app.data

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import androidx.annotation.RequiresApi
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.PublicKey
import java.security.Signature
import java.security.spec.ECGenParameterSpec

data class DeviceSensorIdentity(
    val publicKeyHex: String,
    val signer: SensorSigner,
) {
    init {
        require(publicKeyHex.length == 64 && publicKeyHex.all { it in "0123456789abcdefABCDEF" }) {
            "device Ed25519 public key must be 32-byte hex"
        }
    }
}

/**
 * Owns an Ed25519 sensor key in AndroidKeyStore. Private key bytes are neither
 * returned nor serialized: callers receive only a signing operation.
 *
 * AndroidKeyStore Ed25519 generation is available from API 33. Devices below
 * that level must use the owner-posting fallback until a secure Ed25519 provider
 * is added; silently creating an exportable software key is intentionally not
 * supported here.
 */
@RequiresApi(33)
class AndroidSensorKeyStore(
    private val keyStore: KeyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) },
) {
    fun getOrCreate(sensorId: String): DeviceSensorIdentity {
        val alias = aliasFor(sensorId)
        // An earlier build generated these aliases with KeyPairGenerator("Ed25519"),
        // which AndroidKeyStore silently turned into a P-256 key. Drop any alias whose
        // public key is not Ed25519 so it is regenerated correctly below.
        if (keyStore.containsAlias(alias)) {
            val enc = keyStore.getCertificate(alias)?.publicKey?.encoded
            val isEd25519 = enc != null && enc.size == ED25519_X509_PREFIX.size + 32 &&
                enc.copyOfRange(0, ED25519_X509_PREFIX.size).contentEquals(ED25519_X509_PREFIX)
            if (!isEd25519) keyStore.deleteEntry(alias)
        }
        if (!keyStore.containsAlias(alias)) {
            // AndroidKeyStore exposes Ed25519 (API 33+) as an EC key on the "ed25519" curve.
            KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_EC, KEYSTORE).run {
                initialize(
                    KeyGenParameterSpec.Builder(alias, KeyProperties.PURPOSE_SIGN)
                        .setAlgorithmParameterSpec(ECGenParameterSpec("ed25519"))
                        .setDigests(KeyProperties.DIGEST_NONE)
                        .build(),
                )
                generateKeyPair()
            }
        }

        val privateKey = requireNotNull(keyStore.getKey(alias, null) as? PrivateKey) {
            "AndroidKeyStore did not return the sensor signing key"
        }
        val publicKey = requireNotNull(keyStore.getCertificate(alias)?.publicKey) {
            "AndroidKeyStore did not return the sensor public key"
        }
        return DeviceSensorIdentity(
            publicKeyHex = rawEd25519PublicKey(publicKey).toHex(),
            signer = SensorSigner { message ->
                Signature.getInstance(ED25519).run {
                    initSign(privateKey)
                    update(message)
                    sign()
                }
            },
        )
    }

    companion object {
        private const val KEYSTORE = "AndroidKeyStore"
        // KeyProperties has no Ed25519 constant in the compileSdk 36 stubs; use the
        // JCA algorithm name. NOTE: hardware-backed Ed25519 generation is still the
        // unverified proposal from 08f56602 — not exercised by the observer build.
        private const val ED25519 = "Ed25519"

        private fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
        private const val ALIAS_PREFIX = "hone.sensor.ed25519."
        private val ED25519_X509_PREFIX = byteArrayOf(
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        )

        internal fun aliasFor(sensorId: String): String {
            require(sensorId.isNotBlank()) { "sensorId must not be blank" }
            val safeId = sensorId.toByteArray(Charsets.UTF_8).toHex()
            return ALIAS_PREFIX + safeId
        }

        internal fun rawEd25519PublicKey(publicKey: PublicKey): ByteArray {
            val encoded = publicKey.encoded
            require(encoded.size == ED25519_X509_PREFIX.size + 32 &&
                encoded.copyOfRange(0, ED25519_X509_PREFIX.size).contentEquals(ED25519_X509_PREFIX)) {
                "AndroidKeyStore returned a non-Ed25519 public key"
            }
            return encoded.copyOfRange(ED25519_X509_PREFIX.size, encoded.size)
        }
    }
}

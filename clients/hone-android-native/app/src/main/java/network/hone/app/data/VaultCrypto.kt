package network.hone.app.data

import org.bouncycastle.crypto.generators.Argon2BytesGenerator
import org.bouncycastle.crypto.params.Argon2Parameters
import org.bouncycastle.math.ec.rfc8032.Ed25519
import java.nio.ByteBuffer
import java.nio.CharBuffer
import java.security.SecureRandom
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * Pure-JVM core of the active key vault (unit-tested; no Android types).
 *
 * Two layers:
 *  - inner: AES-256-GCM under a key derived from the user's password with Argon2id
 *    (random 16-byte salt, 64 MiB, 3 passes, 1 lane). The header (magic, version, KDF
 *    parameters, salt, public key) is the GCM AAD, so changing any of it fails the tag.
 *  - outer: AES-256-GCM under an Android Keystore key (hardware-backed, StrongBox where
 *    available). The outer layer is applied by [wrap]/[unwrap] with a Cipher the caller
 *    initialised from the Keystore key, so the file is useless off the phone, and the
 *    password is still needed on it.
 *
 * The decrypted seed only ever lives in a ByteArray that [signOnce] zeroes before returning.
 */
object VaultCrypto {
    private val MAGIC = byteArrayOf('H'.code.toByte(), 'V'.code.toByte(), '2'.code.toByte())
    private const val VERSION: Byte = 2
    private const val SALT = 16
    private const val IV = 12
    private const val TAG_BITS = 128
    private val OUTER_AAD = "hone.vault.outer.v2".toByteArray()

    /** Argon2id cost. Defaults follow RFC 9106's second recommended option, raised to 3 passes. */
    data class Kdf(val memKiB: Int = 64 * 1024, val iterations: Int = 3, val lanes: Int = 1) {
        init { require(memKiB >= 8 * lanes && iterations >= 1 && lanes in 1..16) }
    }

    class WrongPassword : Exception("wrong password")
    class Tampered : Exception("vault data was changed or damaged")

    private val HEX = Regex("[0-9a-f]{64}")

    /** 64 hex chars -> 32-byte seed. Throws IllegalArgumentException otherwise. Caller zeroes it. */
    fun parseSeedHex(hex: String): ByteArray {
        val h = hex.trim().lowercase()
        require(HEX.matches(h)) { "Expected 64 hex characters (0-9, a-f); got ${h.length}" }
        return ByteArray(32) { i -> ((Character.digit(h[i * 2], 16) shl 4) or Character.digit(h[i * 2 + 1], 16)).toByte() }
    }

    fun pubOf(seed: ByteArray): String {
        require(seed.size == 32)
        val pk = ByteArray(Ed25519.PUBLIC_KEY_SIZE)
        Ed25519.generatePublicKey(seed, 0, pk, 0)
        return pk.toHex()
    }

    /** Null when [seed] is the on-chain active key [chainPub]; otherwise why it is refused. */
    fun checkAgainstChain(seed: ByteArray, chainPub: String?): String? = when {
        chainPub.isNullOrBlank() -> "Could not read the account's active key from the chain; not imported"
        pubOf(seed) != chainPub.trim().lowercase() -> "This is not the account's active key (public key does not match the chain); not imported"
        else -> null
    }

    /** Password chars -> UTF-8 bytes without going through a String. Caller zeroes the result. */
    fun utf8(password: CharArray): ByteArray {
        val bb = Charsets.UTF_8.newEncoder().encode(CharBuffer.wrap(password))
        val out = ByteArray(bb.remaining()); bb.get(out)
        if (bb.hasArray()) bb.array().fill(0)
        return out
    }

    private fun derive(password: CharArray, salt: ByteArray, kdf: Kdf): ByteArray {
        val pw = utf8(password)
        try {
            val p = Argon2Parameters.Builder(Argon2Parameters.ARGON2_id).withVersion(Argon2Parameters.ARGON2_VERSION_13)
                .withSalt(salt).withMemoryAsKB(kdf.memKiB).withIterations(kdf.iterations).withParallelism(kdf.lanes).build()
            val out = ByteArray(32)
            Argon2BytesGenerator().apply { init(p) }.generateBytes(pw, out)
            return out
        } finally { pw.fill(0) }
    }

    private fun header(kdf: Kdf, salt: ByteArray, pub: ByteArray): ByteArray =
        ByteBuffer.allocate(3 + 1 + 4 + 4 + 1 + SALT + 32).put(MAGIC).put(VERSION)
            .putInt(kdf.memKiB).putInt(kdf.iterations).put(kdf.lanes.toByte()).put(salt).put(pub).array()
    private const val HEADER = 3 + 1 + 4 + 4 + 1 + SALT + 32

    /** Inner sealed blob: header | iv | AES-GCM(seed) with header as AAD. */
    fun seal(seed: ByteArray, password: CharArray, kdf: Kdf = Kdf(), rnd: SecureRandom = SecureRandom()): ByteArray {
        require(seed.size == 32)
        val salt = ByteArray(SALT).also(rnd::nextBytes)
        val iv = ByteArray(IV).also(rnd::nextBytes)
        val pub = ByteArray(32).also { Ed25519.generatePublicKey(seed, 0, it, 0) }
        val hdr = header(kdf, salt, pub)
        val k = derive(password, salt, kdf)
        try {
            val c = Cipher.getInstance("AES/GCM/NoPadding")
            c.init(Cipher.ENCRYPT_MODE, SecretKeySpec(k, "AES"), GCMParameterSpec(TAG_BITS, iv))
            c.updateAAD(hdr)
            return hdr + iv + c.doFinal(seed)
        } finally { k.fill(0) }
    }

    /** Public key stored in an inner blob's header (no password needed). */
    fun pubOfBlob(blob: ByteArray): String {
        if (blob.size < HEADER + IV + 32 + 16 || !blob.copyOfRange(0, 3).contentEquals(MAGIC) || blob[3] != VERSION) throw Tampered()
        return blob.copyOfRange(HEADER - 32, HEADER).toHex()
    }

    /** Decrypt an inner blob into a new 32-byte seed. The CALLER must zero it. */
    fun open(blob: ByteArray, password: CharArray): ByteArray {
        pubOfBlob(blob)
        val bb = ByteBuffer.wrap(blob, 4, HEADER - 4)
        val kdf = runCatching { Kdf(bb.int, bb.int, bb.get().toInt()) }.getOrElse { throw Tampered() }
        if (kdf.memKiB > 1024 * 1024 || kdf.iterations > 64) throw Tampered()   // refuse absurd costs
        val salt = blob.copyOfRange(13, 13 + SALT)
        val hdr = blob.copyOfRange(0, HEADER)
        val iv = blob.copyOfRange(HEADER, HEADER + IV)
        val k = derive(password, salt, kdf)
        val seed = try {
            val c = Cipher.getInstance("AES/GCM/NoPadding")
            c.init(Cipher.DECRYPT_MODE, SecretKeySpec(k, "AES"), GCMParameterSpec(TAG_BITS, iv))
            c.updateAAD(hdr)
            c.doFinal(blob, HEADER + IV, blob.size - HEADER - IV)
        } catch (e: AEADBadTagException) {
            // The outer Keystore layer already authenticated the file, so a failing inner tag
            // means the password is wrong.
            throw WrongPassword()
        } finally { k.fill(0) }
        if (seed.size != 32 || pubOf(seed) != pubOfBlob(blob)) { seed.fill(0); throw Tampered() }
        return seed
    }

    /** Outer layer with a cipher already initialised for ENCRYPT (Keystore picks the IV). */
    fun wrap(encrypt: Cipher, inner: ByteArray): ByteArray {
        encrypt.updateAAD(OUTER_AAD)
        val ct = encrypt.doFinal(inner)
        val iv = encrypt.iv
        require(iv.size == IV)
        return iv + ct
    }
    fun outerIv(outer: ByteArray): GCMParameterSpec {
        if (outer.size < IV + 16) throw Tampered()
        return GCMParameterSpec(TAG_BITS, outer, 0, IV)
    }
    /** Undo [wrap] with a cipher initialised for DECRYPT with [outerIv]. Throws [Tampered]. */
    fun unwrap(decrypt: Cipher, outer: ByteArray): ByteArray = try {
        decrypt.updateAAD(OUTER_AAD)
        decrypt.doFinal(outer, IV, outer.size - IV)
    } catch (e: AEADBadTagException) { throw Tampered() }

    /**
     * Decrypt the seed, sign [message] once, zero the seed. [seedSpy] is a test hook that
     * receives the plaintext buffer so a test can check it was wiped; production passes nothing.
     * Returns (signature hex, public key hex).
     */
    fun signOnce(inner: ByteArray, password: CharArray, message: ByteArray,
                 seedSpy: ((ByteArray) -> Unit)? = null): Pair<String, String> {
        val seed = open(inner, password)
        try {
            seedSpy?.invoke(seed)
            val pk = ByteArray(Ed25519.PUBLIC_KEY_SIZE)
            Ed25519.generatePublicKey(seed, 0, pk, 0)
            val sig = ByteArray(Ed25519.SIGNATURE_SIZE)
            Ed25519.sign(seed, 0, pk, 0, message, 0, message.size, sig, 0)
            return sig.toHex() to pk.toHex()
        } finally { seed.fill(0) }
    }

    fun verify(pubHex: String, message: ByteArray, sigHex: String): Boolean = runCatching {
        Ed25519.verify(sigHex.hexBytes(), 0, pubHex.hexBytes(), 0, message, 0, message.size)
    }.getOrDefault(false)

    internal fun ByteArray.toHex() = joinToString("") { "%02x".format(it) }
    internal fun String.hexBytes() = ByteArray(length / 2) { i -> substring(i * 2, i * 2 + 2).toInt(16).toByte() }
}

/** Wrong-password backoff: free tries up to [FREE], then 30 s doubling per failure, capped at 1 h. */
object VaultBackoff {
    const val FREE = 5
    fun delayMs(failures: Int): Long =
        if (failures < FREE) 0L else minOf(30_000L shl minOf(failures - FREE, 10), 3_600_000L)
}

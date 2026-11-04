package network.hone.app.sightings

import org.bouncycastle.crypto.params.Ed25519PrivateKeyParameters
import org.bouncycastle.math.ec.rfc7748.X25519
import java.math.BigInteger
import java.nio.ByteBuffer
import java.security.MessageDigest
import java.security.SecureRandom
import java.time.Instant
import java.time.ZoneOffset
import javax.crypto.Cipher
import javax.crypto.Mac
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec

/**
 * Key schedule for owner-trackable rotating identities (spec §1a) and location sharing.
 * Pure JVM, no Android types, so it is unit-tested against the Python tools.
 *
 *   id_seed(device)   = HKDF(ikm = hide_private, salt = "", info = "hone-id-seed" || device_pubkey)
 *   period            = UTC calendar month of the window start, as year*12 + (month-1)
 *   share_k(period)   = HKDF(ikm = id_seed, info = "hone-share" || period_be32)
 *   sk_w              = HKDF(ikm = share_k, info = "hone-pkw" || window_be64)   (X25519 scalar)
 *   pk_w              = X25519(sk_w, 9)                 -> broadcast in the advert
 *   lid               = SHA-256(pk_w)[0..16]            -> lookup id observers file sightings under
 *   tag               = HKDF(share_k, info = "hone-tag" || window_be64)[0..2]  (joins legacy fragments)
 *
 * Holding share_k for a month lets someone (a grantee) derive every pk_w/sk_w of that
 * month and nothing else; holding id_seed gives every month; hide_private gives every device.
 */
object IdKeys {
    const val WINDOW_S = 900L
    private val rnd = SecureRandom()

    fun window(nowMs: Long = System.currentTimeMillis()): Long = Math.floorDiv(nowMs / 1000, WINDOW_S)
    fun windowStartMs(w: Long): Long = w * WINDOW_S * 1000

    fun period(window: Long): Int {
        val d = Instant.ofEpochMilli(windowStartMs(window)).atZone(ZoneOffset.UTC)
        return d.year * 12 + (d.monthValue - 1)
    }

    fun periodWindows(period: Int): LongRange {
        val y = Math.floorDiv(period, 12); val m = Math.floorMod(period, 12) + 1
        val start = java.time.LocalDate.of(y, m, 1).atStartOfDay(ZoneOffset.UTC).toEpochSecond()
        val end = java.time.LocalDate.of(y, m, 1).plusMonths(1).atStartOfDay(ZoneOffset.UTC).toEpochSecond()
        return (start / WINDOW_S) until (end / WINDOW_S)
    }

    // ── primitives ───────────────────────────────────────────────────────────────
    fun hmac(key: ByteArray, vararg parts: ByteArray): ByteArray {
        val m = Mac.getInstance("HmacSHA256")
        m.init(SecretKeySpec(if (key.isEmpty()) ByteArray(32) else key, "HmacSHA256"))
        parts.forEach { m.update(it) }
        return m.doFinal()
    }

    /** RFC 5869 HKDF-SHA256. Empty salt = 32 zero bytes. */
    fun hkdf(ikm: ByteArray, salt: ByteArray, info: ByteArray, len: Int = 32): ByteArray {
        val prk = hmac(salt, ikm)
        val out = ByteArray(len)
        var t = ByteArray(0); var pos = 0; var i = 1
        while (pos < len) {
            t = hmac(prk, t, info, byteArrayOf(i.toByte()))
            val n = minOf(t.size, len - pos); System.arraycopy(t, 0, out, pos, n); pos += n; i++
        }
        return out
    }

    fun sha256(b: ByteArray): ByteArray = MessageDigest.getInstance("SHA-256").digest(b)
    private fun be64(v: Long) = ByteBuffer.allocate(8).putLong(v).array()
    private fun be32(v: Int) = ByteBuffer.allocate(4).putInt(v).array()
    private fun s(x: String) = x.toByteArray(Charsets.US_ASCII)

    // ── schedule ─────────────────────────────────────────────────────────────────
    fun deviceSeed(hidePriv: ByteArray, devicePub: ByteArray): ByteArray =
        hkdf(hidePriv, ByteArray(0), s("hone-id-seed") + devicePub)

    fun shareKey(seed: ByteArray, period: Int): ByteArray =
        hkdf(seed, ByteArray(0), s("hone-share") + be32(period))

    class WindowKey(val window: Long, val sk: ByteArray, val pk: ByteArray, val lid: ByteArray, val tag: ByteArray)

    fun windowKey(shareK: ByteArray, window: Long): WindowKey {
        val sk = hkdf(shareK, ByteArray(0), s("hone-pkw") + be64(window))
        val pk = ByteArray(32).also { X25519.scalarMultBase(sk, 0, it, 0) }
        val tag = hkdf(shareK, ByteArray(0), s("hone-tag") + be64(window), 2)
        return WindowKey(window, sk, pk, lid(pk), tag)
    }

    fun windowKeyFromSeed(seed: ByteArray, window: Long) = windowKey(shareKey(seed, period(window)), window)

    fun lid(pk: ByteArray): ByteArray = sha256(pk).copyOf(16)

    // ── ECIES: X25519 + HKDF + AES-256-GCM ───────────────────────────────────────
    // blob = epk(32) || nonce(12) || AES-GCM(key, nonce, pt, aad)
    // key  = HKDF(X25519(esk, pk), salt = epk || pk, info)
    fun seal(pk: ByteArray, info: String, pt: ByteArray, aad: ByteArray = ByteArray(0)): ByteArray {
        val esk = ByteArray(32).also { rnd.nextBytes(it) }
        val epk = ByteArray(32).also { X25519.scalarMultBase(esk, 0, it, 0) }
        val shared = ByteArray(32)
        require(X25519.calculateAgreement(esk, 0, pk, 0, shared, 0)) { "bad public key" }
        val key = hkdf(shared, epk + pk, s(info))
        val nonce = ByteArray(12).also { rnd.nextBytes(it) }
        val c = Cipher.getInstance("AES/GCM/NoPadding")
        c.init(Cipher.ENCRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(128, nonce))
        c.updateAAD(aad)
        return epk + nonce + c.doFinal(pt)
    }

    /** Returns null when the blob is not for this key (or was tampered with). */
    fun open(sk: ByteArray, pk: ByteArray, info: String, blob: ByteArray, aad: ByteArray = ByteArray(0)): ByteArray? {
        if (blob.size < 32 + 12 + 16) return null
        val epk = blob.copyOfRange(0, 32)
        val shared = ByteArray(32)
        if (!X25519.calculateAgreement(sk, 0, epk, 0, shared, 0)) return null
        val key = hkdf(shared, epk + pk, s(info))
        return runCatching {
            val c = Cipher.getInstance("AES/GCM/NoPadding")
            c.init(Cipher.DECRYPT_MODE, SecretKeySpec(key, "AES"), GCMParameterSpec(128, blob.copyOfRange(32, 44)))
            c.updateAAD(aad)
            c.doFinal(blob, 44, blob.size - 44)
        }.getOrNull()
    }

    // ── ed25519 role keys -> X25519 (memo key encryption, Flipper provisioning) ───
    private val P = BigInteger.ONE.shiftLeft(255).subtract(BigInteger.valueOf(19))

    fun ed25519Pub(priv: ByteArray): ByteArray = Ed25519PrivateKeyParameters(priv, 0).generatePublicKey().encoded

    /** Montgomery u = (1+y)/(1-y) mod p for an Edwards public key. */
    fun edPubToX(edPub: ByteArray): ByteArray {
        val le = edPub.copyOf(); le[31] = (le[31].toInt() and 0x7f).toByte()
        val y = BigInteger(1, le.reversedArray())
        val u = BigInteger.ONE.add(y).multiply(BigInteger.ONE.subtract(y).mod(P).modInverse(P)).mod(P)
        val be = u.toByteArray().let { if (it.size > 32) it.copyOfRange(it.size - 32, it.size) else it }
        return (ByteArray(32 - be.size) + be).reversedArray()
    }

    /** X25519 scalar for an ed25519 seed: SHA-512(seed)[0..32] (X25519 clamps it). */
    fun edPrivToX(edPriv: ByteArray): ByteArray = MessageDigest.getInstance("SHA-512").digest(edPriv).copyOf(32)

    fun xPub(sk: ByteArray): ByteArray = ByteArray(32).also { X25519.scalarMultBase(sk, 0, it, 0) }
}

internal fun ByteArray.hex(): String = joinToString("") { "%02x".format(it) }
internal fun String.unhex(): ByteArray = chunked(2).map { it.toInt(16).toByte() }.toByteArray()

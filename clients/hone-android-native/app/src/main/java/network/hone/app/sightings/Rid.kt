package network.hone.app.sightings

import android.content.Context
import java.io.File
import java.nio.ByteBuffer
import java.security.SecureRandom
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * Seeded rotating identity (spec §1a).
 *
 *   rid = HMAC-SHA256(id_seed, "hone-rid" || window_be64)[0..16], window = floor(unix_utc / 900)
 *
 * id_seed is 32 random bytes generated once, kept in app-private storage next to the
 * witness key (files/hone-node/id_seed, mode 600). It is never logged and never leaves
 * the device. TODO(backup): the spec backs the seed up to the owner's wallet; the app has
 * no owner backup path yet, so there is deliberately no export of any kind.
 */
object Rid {
    const val WINDOW_S = 900L
    private val LABEL = "hone-rid".toByteArray(Charsets.US_ASCII)

    fun window(nowMs: Long = System.currentTimeMillis()): Long = Math.floorDiv(nowMs / 1000, WINDOW_S)

    /** Epoch ms at which [w] starts. */
    fun windowStartMs(w: Long): Long = w * WINDOW_S * 1000

    fun rid(seed: ByteArray, window: Long): ByteArray {
        val mac = Mac.getInstance("HmacSHA256")
        mac.init(SecretKeySpec(seed, "HmacSHA256"))
        mac.update(LABEL)
        mac.update(ByteBuffer.allocate(8).putLong(window).array()) // big-endian
        return mac.doFinal().copyOf(16)
    }

    private fun seedFile(ctx: Context) = File(ctx.filesDir, "hone-node/id_seed")

    @Volatile private var cached: ByteArray? = null

    /** This device's id_seed, created on first use. */
    @Synchronized
    fun seed(ctx: Context): ByteArray {
        cached?.let { return it }
        val f = seedFile(ctx)
        val s = if (f.exists() && f.length() == 32L) f.readBytes() else {
            f.parentFile?.mkdirs()
            val b = ByteArray(32).also { SecureRandom().nextBytes(it) }
            val tmp = File(f.parentFile, "id_seed.tmp")
            tmp.writeBytes(b)
            tmp.setReadable(false, false); tmp.setReadable(true, true)
            tmp.setWritable(false, false); tmp.setWritable(true, true)
            tmp.renameTo(f)
            b
        }
        cached = s
        return s
    }
}

internal fun ByteArray.hex(): String = joinToString("") { "%02x".format(it) }
internal fun String.unhex(): ByteArray = chunked(2).map { it.toInt(16).toByte() }.toByteArray()

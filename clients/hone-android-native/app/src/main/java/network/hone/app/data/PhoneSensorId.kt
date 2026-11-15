package network.hone.app.data

import android.content.Context
import android.os.Build
import java.security.MessageDigest

/**
 * Sensor-id prefix for this phone's NodeService sensors (<prefix>-<channel>, e.g. "pixel-8-1a2b3c4d-accel").
 *
 * The prefix used to default to the model name, so two phones of the same model on one account
 * shared sensor ids (and, with sync before submit, one device head: each would see the other's
 * commits as a history mismatch). New installs get model + the first 8 hex of sha256 of this
 * phone's own AndroidKeyStore key, unique per phone. A prefix already saved in
 * sensor_settings.json is never changed, so phones whose sensors are enrolled keep their ids.
 */
object PhoneSensorId {
    /** AndroidKeyStore alias (via AndroidSensorKeyStore) of the key the prefix is derived from. */
    const val KEY_ID = "hone-phone-id"

    /** "<model>-<sha256(public key)[0..4] hex>". [publicKeyHex] is the raw 32-byte key in hex. */
    fun prefix(model: String, publicKeyHex: String): String {
        val m = model.lowercase().replace(Regex("[^a-z0-9]+"), "-").trim('-').take(32).ifEmpty { "phone" }
        val key = publicKeyHex.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
        require(key.size == 32) { "public key must be 32 bytes" }
        val h = MessageDigest.getInstance("SHA-256").digest(key).joinToString("") { "%02x".format(it) }
        return "$m-${h.take(8)}"
    }

    /** The prefix to use: the saved one if there is any (keeps enrolled ids), else the per-phone form. */
    fun resolve(saved: String?, model: String, publicKeyHex: () -> String?): String {
        saved?.trim()?.takeIf { it.isNotEmpty() }?.let { return it }
        val key = runCatching { publicKeyHex() }.getOrNull()
        return if (key != null) prefix(model, key) else legacy(model)
    }

    /** The old default (model name only); used only when no phone key is available at all. */
    fun legacy(model: String): String = model.lowercase().replace(' ', '-')

    /** This phone's public key: its AndroidKeyStore key (API 33+), else the seat key. */
    fun phoneKeyHex(ctx: Context): String? =
        (if (Build.VERSION.SDK_INT >= 33) runCatching { AndroidSensorKeyStore().getOrCreate(KEY_ID).publicKeyHex }.getOrNull() else null)
            ?: runCatching { network.hone.app.telemetry.Telemetry.seatPub(ctx) }.getOrNull()?.takeIf { it.length == 64 }

    /** The prefix for this install, given what sensor_settings.json holds ([saved], may be null). */
    fun forThisPhone(ctx: Context, saved: String?): String = resolve(saved, Build.MODEL) { phoneKeyHex(ctx) }
}

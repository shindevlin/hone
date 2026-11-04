package network.hone.app.sound

import android.content.Context
import android.util.Log
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * Individual bird voiceprints (crates/hone-voiceprint, JNI build: libhone_voiceprint.so).
 *
 * Feed it every detection that already passed the human-speech gate (SoundSensing drops frames
 * with any human class above HUMAN_MAX before this point). It keeps only 64-value int8 prints,
 * never audio, in filesDir/hone-node/sound/voiceprints.json, which never leaves the phone.
 *
 * Events (drainEvents):
 *  - "nameable": a candidate individual first passed the naming gate -> submit chain
 *    BIRD_OBSERVE {bird_id} (first reporter = this device's seat), offer "Name this bird".
 *  - "back": show "<name> is back, night <nights_seen>" with phrase(likelihood).
 *  - "merged": from -> into (earliest name wins); re-point any local UI state.
 *  - "split", "candidate": informational.
 * The naming account is the device owner's account (no pseudonyms).
 */
class Voiceprints private constructor(private val file: File, private var h: Long) : AutoCloseable {

    /** Returns the match JSON ({bird_id, likelihood, new_cluster}) or null if ignored. */
    @Synchronized
    fun observe(species: String, embedding: FloatArray, tMs: Long, conf: Float): JSONObject? =
        if (h == 0L) null else nativeObserve(h, species, embedding, tMs, conf)?.let { JSONObject(it) }

    @Synchronized
    fun setLocation(lat: Double, lon: Double, tzMin: Int): Boolean = h != 0L && nativeSetLocation(h, lat, lon, tzMin)

    @Synchronized
    fun drainEvents(): JSONArray = JSONArray(if (h == 0L) "[]" else nativeQuery(h, 0, 0, null) ?: "[]")

    @Synchronized
    fun candidates(): JSONArray = JSONArray(if (h == 0L) "[]" else nativeQuery(h, 1, 0, null) ?: "[]")

    /** {bird_id, species, nights_seen, last_heard, name, naming_allowed, heard_tonight, back_likelihood, is_back} */
    @Synchronized
    fun status(birdId: String): JSONObject? = if (h == 0L) null else nativeQuery(h, 3, 0, birdId)?.let { JSONObject(it) }

    /** Summary for the 4 KB device report: bird ids and counts only. */
    @Synchronized
    fun report(maxBytes: Int): String = if (h == 0L) "{}" else nativeQuery(h, 2, maxBytes, null) ?: "{}"

    /** Local label; null on success, else the reason (e.g. species not offered for naming). */
    @Synchronized
    fun name(birdId: String, name: String): String? = if (h == 0L) "closed" else nativeName(h, birdId, name)

    @Synchronized
    fun save() {
        if (h == 0L) return
        val b = nativeSave(h) ?: return
        runCatching {
            val tmp = File(file.parentFile, file.name + ".tmp")
            tmp.writeBytes(b)
            tmp.renameTo(file)
        }.onFailure { Log.w(TAG, "save failed", it) }
    }

    @Synchronized
    override fun close() {
        if (h != 0L) { save(); nativeFree(h); h = 0L }
    }

    companion object {
        private const val TAG = "Voiceprints"
        private val loaded = runCatching { System.loadLibrary("hone_voiceprint") }.isSuccess

        /** Null if the native library is missing (feature off). */
        fun open(ctx: Context, conf: JSONObject = JSONObject()): Voiceprints? {
            if (!loaded) return null
            val f = File(ctx.filesDir, "hone-node/sound/voiceprints.json").apply { parentFile?.mkdirs() }
            val state = if (f.exists()) runCatching { f.readBytes() }.getOrNull() else null
            val h = nativeNew(conf.toString(), state)
            return if (h == 0L) null else Voiceprints(f, h)
        }

        fun phrase(p: Double): String = when {
            p >= 0.9 -> "very likely"
            p >= 0.7 -> "likely"
            p >= 0.5 -> "possibly"
            else -> "unsure"
        }

        @JvmStatic private external fun nativeNew(cfg: String, state: ByteArray?): Long
        @JvmStatic private external fun nativeFree(h: Long)
        @JvmStatic private external fun nativeSetLocation(h: Long, lat: Double, lon: Double, tzMin: Int): Boolean
        @JvmStatic private external fun nativeObserve(h: Long, species: String, emb: FloatArray, t: Long, conf: Float): String?
        @JvmStatic private external fun nativeQuery(h: Long, kind: Int, arg: Int, id: String?): String?
        @JvmStatic private external fun nativeName(h: Long, id: String, name: String): String?
        @JvmStatic private external fun nativeSave(h: Long): ByteArray?
    }
}

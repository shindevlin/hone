package network.hone.app.birds

import android.content.Context
import network.hone.app.sound.Detection
import org.json.JSONObject
import java.io.File

/** Caught birds (Ornigo encounters), kept on the phone in hone-node/sound/catches.jsonl. */
object Catches {
    /** Same bar as "likely": a call must be at least this sure to fill the ring. */
    const val CATCH_MIN = 0.70f
    /** Cosine similarity above which two calls are the same sound played again (a recording). */
    private const val REPLAY_SIM = 0.985f

    private fun file(ctx: Context) = File(ctx.filesDir, "hone-node/sound/catches.jsonl").apply { parentFile?.mkdirs() }

    fun record(ctx: Context, latin: String, conf: Float) {
        runCatching { file(ctx).appendText(JSONObject().put("t", System.currentTimeMillis()).put("name", latin).put("conf", conf.toDouble()).toString() + "\n") }
    }

    data class Catch(val t: Long, val name: String)

    fun all(ctx: Context): List<Catch> {
        val f = file(ctx); if (!f.exists()) return emptyList()
        return f.readLines().mapNotNull { l -> runCatching { JSONObject(l).let { Catch(it.getLong("t"), it.getString("name")) } }.getOrNull() }
    }

    fun count(ctx: Context, latin: String) = all(ctx).count { it.name == latin }

    /**
     * Replay check. Live birds never repeat a call sample-for-sample, so their Perch embeddings
     * differ a little every time; a recording played on a loop gives near-identical embeddings.
     */
    fun looksReplayed(calls: List<Detection>): Boolean {
        // Any two calls that are the same sound sample for sample give it away.
        val e = calls.mapNotNull { it.emb }
        for (i in e.indices) for (j in i + 1 until e.size) if (cos(e[i], e[j]) > REPLAY_SIM) return true
        return false
    }

    private fun cos(a: FloatArray, b: FloatArray): Float {
        var d = 0.0; var na = 0.0; var nb = 0.0
        for (i in a.indices) { d += a[i] * b[i]; na += a[i] * a[i]; nb += b[i] * b[i] }
        return if (na == 0.0 || nb == 0.0) 0f else (d / Math.sqrt(na * nb)).toFloat()
    }
}

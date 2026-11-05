package network.hone.app.sound

import android.content.Context
import android.util.Log
import network.hone.app.sightings.SightingsEngine
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * Voiceprints in shadow mode (Josh's 14-night kiskadee test). Every detection that passed the
 * human-speech gate is fed to [Voiceprints]; state lives in filesDir/hone-node/sound/voiceprints.json.
 * Shadow: nothing is shown, no BIRD_OBSERVE is sent, nothing is offered for naming. The only way
 * out is the hidden export under Settings > Advanced (candidates, nights, likelihoods; no prints).
 */
object VoiceprintShadow {
    private const val TAG = "VoiceprintShadow"
    private const val LOG_MAX = 4000
    private const val SAVE_EVERY_MS = 5 * 60_000L
    private const val LOC_EVERY_MS = 10 * 60_000L

    @Volatile private var vp: Voiceprints? = null
    @Volatile private var tried = false
    private var lastSave = 0L
    private var lastLoc = 0L
    private var located = false
    private var logLines = -1

    private fun open(ctx: Context): Voiceprints? {
        vp?.let { return it }
        if (tried) return null
        tried = true
        vp = runCatching { Voiceprints.open(ctx) }.onFailure { Log.w(TAG, "open failed", it) }.getOrNull()
        Log.i(TAG, "voiceprints ${if (vp != null) "on (shadow)" else "unavailable"}")
        return vp
    }

    private fun logFile(ctx: Context) = File(ctx.filesDir, "hone-node/sound/voiceprint-shadow.jsonl")

    @Synchronized
    fun observe(ctx: Context, species: String, emb: FloatArray, tMs: Long, conf: Float) {
        val v = open(ctx) ?: return
        runCatching {
            val now = System.currentTimeMillis()
            if (!located || now - lastLoc > LOC_EVERY_MS) SightingsEngine.lastFix()?.let {
                val tz = java.util.TimeZone.getDefault().getOffset(now) / 60_000
                located = v.setLocation(it.latitude, it.longitude, tz) || located
                lastLoc = now
            }
            val m = v.observe(species, emb, tMs, conf)
            val sb = StringBuilder()
            if (m != null) sb.append(JSONObject().put("t", tMs).put("sp", species).put("conf", Math.round(conf * 1000) / 1000.0)
                .put("bird_id", m.opt("bird_id")).put("likelihood", m.opt("likelihood")).put("new_cluster", m.opt("new_cluster"))
                .toString()).append('\n')
            // Events are drained so they don't pile up, and only logged: shadow mode acts on none of them.
            val ev = v.drainEvents()
            for (i in 0 until ev.length()) sb.append(JSONObject().put("t", now).put("event", ev.get(i)).toString()).append('\n')
            if (sb.isNotEmpty()) appendLog(ctx, sb.toString())
            if (now - lastSave > SAVE_EVERY_MS) { v.save(); lastSave = now }
        }.onFailure { Log.w(TAG, "observe failed", it) }
    }

    private fun appendLog(ctx: Context, s: String) {
        val f = logFile(ctx).apply { parentFile?.mkdirs() }
        f.appendText(s)
        if (logLines < 0) logLines = runCatching { f.useLines { it.count() } }.getOrDefault(0) else logLines += s.count { it == '\n' }
        if (logLines > LOG_MAX * 2) {
            val keep = f.readLines().takeLast(LOG_MAX)
            f.writeText(keep.joinToString("\n", postfix = "\n")); logLines = keep.size
        }
    }

    /** The debug export: candidates (nights, last heard, likelihoods) and recent matches. No voiceprints. */
    @Synchronized
    fun export(ctx: Context): String {
        val v = open(ctx)
        v?.save()
        val recent = JSONArray()
        runCatching { logFile(ctx).takeIf { it.exists() }?.readLines()?.takeLast(1000)?.forEach { recent.put(JSONObject(it)) } }
        return JSONObject()
            .put("mode", "shadow")
            .put("exported_at", System.currentTimeMillis())
            .put("library", v != null)
            .put("located", located)
            .put("candidates", v?.candidates() ?: JSONArray())
            .put("recent", recent)
            .toString(2)
    }
}

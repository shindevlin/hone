package network.hone.app.sound

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.os.BatteryManager
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import network.hone.app.sightings.Cells
import network.hone.app.sightings.SightingsEngine
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest
import kotlin.math.log10
import kotlin.math.max

data class Detection(val name: String, val conf: Float, val atMs: Long)

data class SoundLive(
    val enabled: Boolean = false,
    val listening: Boolean = false,
    val status: String = "",
    val framesHeard: Int = 0,
    val framesHuman: Int = 0,
    val lastLeq: Double? = null,
    val lastTop: List<Detection> = emptyList(),
    val windowSpecies: List<Detection> = emptyList(),
    val recent: List<Detection> = emptyList(),
    val reportsStored: Int = 0,
)

/**
 * Sound sensing (sightings spec §2d/§2e/§2f): bird check + ambient noise, on the phone.
 *
 * Privacy rules this file enforces:
 *  - Audio lives only in RAM, one 5 s frame at a time, and is zeroed after use. Never written.
 *  - Perch classifies the frame first. If any human class (speech, singing, crowd…) is present,
 *    the whole frame is dropped: no levels, no species, only a counter goes up.
 *  - What leaves this object is a per-window report: species names with confidence, noise
 *    levels in dBFS, listening seconds and the dropped-frame count. Nothing else.
 *
 * Off by default. Enabled only after the user accepts the notice (see [NOTICE]).
 */
object SoundSensing {
    private const val TAG = "HoneSound"
    private const val PREFS = "hone_sound"
    const val NOTICE_VERSION = 1
    const val CODE_URL = "https://github.com/shindevlin/hone/tree/main/clients/hone-android-native/app/src/main/java/network/hone/app/sound"
    val NOTICE = """
        Check your local laws before turning on Sound sensing.

        Sound sensing (bird check and ambient noise) uses your microphone and listens regularly in the background.

        We built it to discard human speech: voices are detected and dropped on your phone, and no audio is stored or sent. Only noise levels and species leave the device.

        You can verify this yourself. The code is open source:
        $CODE_URL

        Some places restrict recording, even when no voice is kept. You are responsible for checking the law where you use HONE.
    """.trimIndent()

    private const val SPECIES_MIN = 0.50f      // softmax prob to count a species (raised from 0.30, Josh)
    private const val HUMAN_MAX = 0.05f        // any human class above this drops the frame
    private const val FRAMES_PER_BURST = 2     // 10 s of listening …
    private const val BURST_EVERY_MS = 60_000L // … every minute on battery; continuous on power
    private const val WINDOW_MS = 900_000L     // UTC quarter-hours, same as sightings

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var job: Job? = null
    private var perch: Perch? = null

    private val _live = MutableStateFlow(SoundLive())
    val live: StateFlow<SoundLive> = _live.asStateFlow()

    // Reports waiting for NodeService to commit (hash on chain, report kept on the device).
    private val pending = ArrayDeque<JSONObject>()

    fun isEnabled(ctx: Context) = prefs(ctx).getBoolean("enabled", false) &&
        prefs(ctx).getInt("notice_version", 0) >= NOTICE_VERSION

    fun hasMic(ctx: Context) =
        ctx.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    /** Called after the user ticked the notice box. Stores the acknowledgment on the device. */
    fun acceptNoticeAndEnable(ctx: Context) {
        prefs(ctx).edit()
            .putInt("notice_version", NOTICE_VERSION)
            .putLong("notice_accepted_ms", System.currentTimeMillis())
            .putBoolean("enabled", true)
            .apply()
        start(ctx)
    }

    fun disable(ctx: Context) {
        prefs(ctx).edit().putBoolean("enabled", false).apply()
        stop()
    }

    fun start(ctx: Context) {
        val app = ctx.applicationContext
        appCtx = app
        if (!isEnabled(app)) { _live.value = SoundLive(enabled = false); return }
        if (job?.isActive == true) return
        _live.value = _live.value.copy(enabled = true, status = "starting…")
        job = scope.launch { runCatching { loop(app) }.onFailure {
            Log.e(TAG, "sound loop failed", it)
            _live.value = _live.value.copy(listening = false, status = "error: ${it.message}")
        } }
    }

    fun stop() {
        job?.cancel(); job = null
        _live.value = _live.value.copy(enabled = false, listening = false, status = "off")
    }

    /** NodeService takes finished window reports to commit. */
    @Synchronized fun drainReports(): List<JSONObject> = pending.toList().also { pending.clear() }

    // ── loop ────────────────────────────────────────────────────────────────────

    private class Window(val index: Long) {
        val species = HashMap<String, Pair<Float, Int>>()   // name -> (max conf, frames)
        var frames = 0
        var framesHuman = 0
        val blockDb = ArrayList<Double>()                   // 100 ms block levels, non-human frames
        var peak = -200.0
    }

    private suspend fun loop(ctx: Context) {
        if (!hasMic(ctx)) { _live.value = _live.value.copy(status = "microphone permission needed"); return }
        val p = perch ?: runCatching { Perch(ctx) }.getOrElse {
            _live.value = _live.value.copy(status = "model missing: ${it.message}"); return
        }.also { perch = it }

        var win = Window(System.currentTimeMillis() / WINDOW_MS)
        val frame = FloatArray(Perch.FRAME)
        while (currentCoroutineContext().isActive) {
            val burst = if (onPower(ctx)) 6 else FRAMES_PER_BURST
            val started = System.currentTimeMillis()
            for (i in 0 until burst) {
                val now = System.currentTimeMillis()
                if (now / WINDOW_MS != win.index) { closeWindow(ctx, win); win = Window(now / WINDOW_MS) }
                _live.value = _live.value.copy(listening = true, status = "listening")
                if (!record(frame)) { _live.value = _live.value.copy(status = "mic busy or blocked"); break }
                if (frame.all { it == 0f }) {
                    _live.value = _live.value.copy(listening = false, status = "microphone silenced by Android: open HONE to resume")
                    break
                }
                processFrame(p, frame, win)
                frame.fill(0f)
            }
            _live.value = _live.value.copy(listening = false, status = "waiting")
            if (!onPower(ctx)) delay(max(0L, BURST_EVERY_MS - (System.currentTimeMillis() - started)))
        }
    }

    private fun processFrame(p: Perch, frame: FloatArray, win: Window) {
        val probs = p.classify(frame)
        win.frames++
        val human = p.humanIdx.maxOfOrNull { probs[it] } ?: 0f
        if (human > HUMAN_MAX) {
            win.framesHuman++
            _live.value = _live.value.copy(
                framesHeard = _live.value.framesHeard + 1,
                framesHuman = _live.value.framesHuman + 1,
            )
            return
        }
        // Noise levels, 100 ms blocks, dBFS (uncalibrated; per-device offset comes later).
        val block = Perch.SAMPLE_RATE / 10
        var sumE = 0.0
        for (b in 0 until Perch.FRAME / block) {
            var e = 0.0
            for (k in b * block until (b + 1) * block) { val v = frame[k].toDouble(); e += v * v }
            val db = 10 * log10(e / block + 1e-12)
            win.blockDb.add(db); sumE += e
        }
        var pk = 0f
        for (v in frame) pk = max(pk, kotlin.math.abs(v))
        win.peak = max(win.peak, 20 * log10(pk.toDouble() + 1e-9))
        val leq = 10 * log10(sumE / Perch.FRAME + 1e-12)

        val now = System.currentTimeMillis()
        val ctx = appCtx
        val top = probs.indices.filter {
            p.isSpecies[it] && probs[it] >= SPECIES_MIN &&
                (ctx == null || network.hone.app.birds.Range.allowed(ctx, p.names[it], probs[it]))
        }
            .sortedByDescending { probs[it] }.take(5)
            .map { Detection(p.names[it], probs[it], now) }
        for (d in top) {
            val prev = win.species[d.name]
            win.species[d.name] = Pair(max(prev?.first ?: 0f, d.conf), (prev?.second ?: 0) + 1)
        }
        if (top.isNotEmpty()) logDetections(top, p.lastEmbedding)
        val s = _live.value
        _live.value = s.copy(
            framesHeard = s.framesHeard + 1,
            lastLeq = leq,
            lastTop = top,
            windowSpecies = win.species.map { (k, v) -> Detection(k, v.first, now) }.sortedByDescending { it.conf },
            recent = (top + s.recent).take(20),
        )
    }

    private fun closeWindow(ctx: Context, w: Window) {
        if (w.frames == 0) return
        val sorted = w.blockDb.sorted()
        fun pct(q: Double) = if (sorted.isEmpty()) null else sorted[((1 - q) * (sorted.size - 1)).toInt()]
        val leq = if (w.blockDb.isEmpty()) null
            else 10 * log10(w.blockDb.sumOf { Math.pow(10.0, it / 10) } / w.blockDb.size)
        val fix = SightingsEngine.lastFix()
        val cell = fix?.let { Cells.cell(it.latitude, it.longitude, 9) }
        val r = JSONObject().apply {
            put("v", 1)
            put("window", w.index)
            put("cell", cell ?: JSONObject.NULL)
            put("model", Perch.MODEL_ID)
            put("model_sha256", Perch.MODEL_SHA256)
            put("listen_seconds", w.frames * 5)
            put("frames", w.frames)
            put("frames_discarded_human", w.framesHuman)
            put("species", JSONArray().apply {
                w.species.entries.sortedByDescending { it.value.first }.forEach { (k, v) ->
                    put(JSONObject().put("name", k).put("conf_q8", (v.first * 255).toInt()).put("frames", v.second))
                }
            })
            put("noise", JSONObject().apply {
                put("unit", "dBFS")
                leq?.let { put("leq", round1(it)) }
                pct(0.10)?.let { put("l10", round1(it)) }
                pct(0.50)?.let { put("l50", round1(it)) }
                pct(0.90)?.let { put("l90", round1(it)) }
                if (w.peak > -199) put("peak", round1(w.peak))
            })
        }
        runCatching {
            val dir = File(ctx.filesDir, "hone-node/sound").apply { mkdirs() }
            File(dir, "reports.jsonl").appendText(r.toString() + "\n")
        }
        synchronized(this) { pending.addLast(r); while (pending.size > 96) pending.removeFirst() }
        _live.value = _live.value.copy(reportsStored = _live.value.reportsStored + 1)
        Log.i(TAG, "window ${w.index}: ${w.frames} frames, ${w.framesHuman} dropped, ${w.species.size} species")
    }

    @Volatile private var appCtx: Context? = null

    /** Private detection history for the Birds tab: time, species, confidence, fix. Device only. */
    private fun logDetections(top: List<Detection>, emb: FloatArray) {
        val ctx = appCtx ?: return
        val fix = SightingsEngine.lastFix()
        runCatching {
            // Perch's embedding of this frame, kept on the phone for training a local layer once the
            // user confirms or rejects the detection. Not audio: it can't be played back.
            val ed = File(ctx.filesDir, "hone-node/sound/emb").apply { mkdirs() }
            val bb = java.nio.ByteBuffer.allocate(4 * emb.size).order(java.nio.ByteOrder.LITTLE_ENDIAN)
            bb.asFloatBuffer().put(emb)
            File(ed, "${top[0].atMs}.f32").writeBytes(bb.array())
            val f = File(ctx.filesDir, "hone-node/sound/detections.jsonl").apply { parentFile?.mkdirs() }
            val sb = StringBuilder()
            for (d in top) {
                val j = JSONObject().put("t", d.atMs).put("name", d.name).put("conf", round1(d.conf * 100.0))
                if (network.hone.app.birds.Range.isRare(ctx, d.name)) j.put("rare", true)
                if (fix != null) j.put("lat", fix.latitude).put("lon", fix.longitude).put("acc", fix.accuracy.toDouble())
                sb.append(j.toString()).append('\n')
            }
            f.appendText(sb.toString())
        }
    }

    /** Records one frame into [out]. Returns false if the mic could not be opened. */
    private fun record(out: FloatArray): Boolean {
        val rate = Perch.SAMPLE_RATE
        val minBuf = AudioRecord.getMinBufferSize(rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_FLOAT)
        if (minBuf <= 0) return false
        val rec = runCatching {
            @Suppress("MissingPermission")
            AudioRecord(
                MediaRecorder.AudioSource.UNPROCESSED, rate, AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_FLOAT, max(minBuf, rate * 4),
            ).takeIf { it.state == AudioRecord.STATE_INITIALIZED } ?: AudioRecord(
                MediaRecorder.AudioSource.MIC, rate, AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_FLOAT, max(minBuf, rate * 4),
            )
        }.getOrNull() ?: return false
        try {
            if (rec.state != AudioRecord.STATE_INITIALIZED) return false
            rec.startRecording()
            var off = 0
            while (off < out.size) {
                val n = rec.read(out, off, out.size - off, AudioRecord.READ_BLOCKING)
                if (n <= 0) return false
                off += n
            }
            return true
        } finally {
            runCatching { rec.stop() }; rec.release()
        }
    }

    private fun onPower(ctx: Context): Boolean {
        val i: Intent? = ctx.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        return (i?.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) ?: 0) != 0
    }

    private fun round1(d: Double) = Math.round(d * 10) / 10.0

    private fun prefs(ctx: Context) = ctx.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    fun reportHash(r: JSONObject): String =
        MessageDigest.getInstance("SHA-256").digest(r.toString().toByteArray())
            .joinToString("") { "%02x".format(it) }
}

package network.hone.app.station

import android.content.Context
import android.media.MediaCodec
import android.media.MediaFormat
import android.util.Log
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import network.hone.app.sound.Perch
import network.hone.app.sound.SoundSensing
import network.hone.app.telemetry.Vault
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.concurrent.ArrayBlockingQueue
import kotlin.math.log10
import kotlin.math.max

/** A camera set up as a bird station. Holds no credentials (those are sealed separately). */
data class Station(
    val id: String, val label: String, val host: String, val port: Int, val path: String, val onvif: String?,
    val codec: String, val rate: Int, val tier: String, val cell: String?, val idSource: String,
    val enabled: Boolean, val addedMs: Long,
) {
    fun json(): JSONObject = JSONObject().put("id", id).put("label", label).put("host", host).put("port", port)
        .put("path", path).put("onvif", onvif ?: JSONObject.NULL).put("codec", codec).put("rate", rate).put("tier", tier)
        .put("cell", cell ?: JSONObject.NULL).put("id_source", idSource).put("enabled", enabled).put("added_ms", addedMs)

    companion object {
        fun of(j: JSONObject) = Station(j.getString("id"), j.optString("label"), j.getString("host"), j.getInt("port"),
            j.getString("path"), j.optString("onvif").takeIf { it.isNotEmpty() && it != "null" }, j.optString("codec"),
            j.optInt("rate"), j.optString("tier", "C"), j.optString("cell").takeIf { it.isNotEmpty() && it != "null" },
            j.optString("id_source"), j.optBoolean("enabled", true), j.optLong("added_ms"))
    }
}

/** Live state for the UI and the device status. */
data class StationLive(
    val running: Boolean = false, val reachable: Boolean? = null, val lastError: String? = null,
    val frames: Long = 0, val speechDropped: Long = 0, val birds: Long = 0, val lastFrameMs: Long = 0,
    val kbps: Int = 0, val busyDropped: Long = 0,
)

object StationStore {
    private const val CREDS = "hone.station.creds"
    fun dir(ctx: Context) = File(ctx.filesDir, "stations").apply { mkdirs() }
    fun dataDir(ctx: Context, id: String) = File(dir(ctx), id).apply { mkdirs() }

    fun list(ctx: Context): List<Station> = dir(ctx).listFiles { f -> f.extension == "json" }.orEmpty()
        .mapNotNull { runCatching { Station.of(JSONObject(it.readText())) }.getOrNull() }.sortedBy { it.addedMs }

    fun save(ctx: Context, s: Station) { File(dir(ctx), "${s.id}.json").writeText(s.json().toString()) }

    fun remove(ctx: Context, id: String) {
        File(dir(ctx), "$id.json").delete(); File(dir(ctx), "$id.creds").delete()
        dataDir(ctx, id).deleteRecursively()
    }

    /** Camera login, sealed with a non-exportable Keystore key. Never logged. */
    fun saveCreds(ctx: Context, id: String, user: String, pass: String) {
        File(dir(ctx), "$id.creds").writeBytes(Vault.seal(CREDS, JSONObject().put("u", user).put("p", pass).toString().toByteArray()))
    }

    fun creds(ctx: Context, id: String): Pair<String, String>? {
        val f = File(dir(ctx), "$id.creds"); if (!f.exists()) return null
        val j = Vault.open(CREDS, f.readBytes())?.let { runCatching { JSONObject(String(it)) }.getOrNull() } ?: return null
        return j.optString("u") to j.optString("p")
    }

    /** MAC from the kernel ARP table, when Android still exposes it (older versions). */
    fun arpMac(host: String): String? = runCatching {
        File("/proc/net/arp").readLines().drop(1).map { it.trim().split(Regex("\\s+")) }
            .firstOrNull { it.size >= 4 && it[0] == host && it[3] != "00:00:00:00:00:00" }?.get(3)?.lowercase()
    }.getOrNull()
}

object Stations {
    private val _live = MutableStateFlow<Map<String, StationLive>>(emptyMap())
    val live: StateFlow<Map<String, StationLive>> = _live.asStateFlow()

    @Synchronized fun update(id: String, f: (StationLive) -> StationLive) {
        _live.value = _live.value + (id to f(_live.value[id] ?: StationLive()))
    }
}

/** AAC (mpeg4-generic) via MediaCodec, mono output. */
class AacDecoder(media: SdpMedia) : AudioDecoder {
    private val cfg = media.fmtp["config"] ?: error("AAC track without config")
    private val rc = Rtp.aacConfig(cfg) ?: error("bad AAC config")
    override val rate = rc.first
    private val codec = MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_AUDIO_AAC).apply {
        val f = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, rc.first, rc.second)
        f.setByteBuffer("csd-0", java.nio.ByteBuffer.wrap(cfg.chunked(2).map { it.toInt(16).toByte() }.toByteArray()))
        configure(f, null, null, 0); start()
    }
    private val info = MediaCodec.BufferInfo()
    private var outChannels = rc.second

    override fun decode(payload: ByteArray): FloatArray {
        val i = codec.dequeueInputBuffer(10_000)
        if (i >= 0) {
            codec.getInputBuffer(i)!!.apply { clear(); put(payload) }
            codec.queueInputBuffer(i, 0, payload.size, System.nanoTime() / 1000, 0)
        }
        var out = FloatArray(0)
        while (true) {
            val o = codec.dequeueOutputBuffer(info, 0)
            if (o == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) { outChannels = codec.outputFormat.getInteger(MediaFormat.KEY_CHANNEL_COUNT); continue }
            if (o < 0) break
            val b = codec.getOutputBuffer(o)!!.order(java.nio.ByteOrder.LITTLE_ENDIAN).asShortBuffer()
            val n = b.remaining() / outChannels.coerceAtLeast(1)
            val f = FloatArray(n) { k -> var s = 0f; for (c in 0 until outChannels) s += b.get(k * outChannels + c) / 32768f; s / outChannels }
            out += f
            codec.releaseOutputBuffer(o, false)
        }
        return out
    }

    override fun close() { runCatching { codec.stop() }; runCatching { codec.release() } }
}

/**
 * One Perch for all stations (they run only while charging). Frames queue here; a full
 * queue drops the oldest (counted), so a slow phone degrades instead of piling up audio.
 */
object StationAnalyzer {
    private val q = ArrayBlockingQueue<Pair<StationPipeline, FloatArray>>(3)
    @Volatile private var worker: Thread? = null
    @Volatile private var perch: Perch? = null

    fun submit(ctx: Context, p: StationPipeline, frame: FloatArray) {
        ensure(ctx)
        while (!q.offer(p to frame)) q.poll()?.first?.busyDropped()
    }

    @Synchronized private fun ensure(ctx: Context) {
        if (worker?.isAlive == true) return
        worker = Thread({
            try {
                val pr = perch ?: Perch(ctx.applicationContext).also { perch = it }
                while (!Thread.currentThread().isInterrupted) {
                    val (p, f) = q.take()
                    runCatching { p.analyze(pr, f) }.onFailure { Log.w("HoneStation", "analyze: ${it.javaClass.simpleName}") }
                }
            } catch (_: InterruptedException) {}
        }, "hone-station-perch").apply { isDaemon = true; priority = Thread.MIN_PRIORITY; start() }
    }

    @Synchronized fun stop() { worker?.interrupt(); worker = null; q.clear(); perch?.close(); perch = null }
}

/**
 * The phone mic's pipeline, for one camera: resample to 32 kHz, 5 s frames, Perch, speech
 * gate (any human class drops the whole frame), species, UTC 15-min reports. No audio is
 * ever written or sent: frames live only in memory until classified.
 */
class StationPipeline(private val ctx: Context, private val st: Station, inRate: Int) {
    private val rs = LinearResampler(inRate, Perch.SAMPLE_RATE)
    private val buf = FloatArray(Perch.FRAME)
    private var fill = 0
    private var win = Win(System.currentTimeMillis() / WINDOW_MS)

    private class Win(val index: Long) {
        var frames = 0; var human = 0; val species = HashMap<String, Pair<Float, Int>>()
        val blockDb = ArrayList<Double>(); var peak = -200.0
    }

    fun feed(pcm: FloatArray) {
        val x = rs.process(pcm)
        var i = 0
        while (i < x.size) {
            val n = minOf(x.size - i, buf.size - fill)
            x.copyInto(buf, fill, i, i + n); fill += n; i += n
            if (fill == buf.size) { StationAnalyzer.submit(ctx, this, buf.copyOf()); fill = 0; buf.fill(0f) }
        }
    }

    fun busyDropped() = Stations.update(st.id) { it.copy(busyDropped = it.busyDropped + 1) }

    @Synchronized
    fun analyze(p: Perch, frame: FloatArray) {
        val now = System.currentTimeMillis()
        if (now / WINDOW_MS != win.index) { closeWindow(win); win = Win(now / WINDOW_MS) }
        val probs = p.classify(frame)
        win.frames++
        val human = p.humanIdx.maxOfOrNull { probs[it] } ?: 0f
        if (human > SoundSensing.HUMAN_MAX) {
            win.human++; frame.fill(0f)
            Stations.update(st.id) { it.copy(frames = it.frames + 1, speechDropped = it.speechDropped + 1, lastFrameMs = now) }
            return
        }
        val block = Perch.SAMPLE_RATE / 10
        for (b in 0 until Perch.FRAME / block) {
            var e = 0.0
            for (k in b * block until (b + 1) * block) { val v = frame[k].toDouble(); e += v * v }
            win.blockDb.add(10 * log10(e / block + 1e-12))
        }
        var pk = 0f; for (v in frame) pk = max(pk, kotlin.math.abs(v))
        win.peak = max(win.peak, 20 * log10(pk.toDouble() + 1e-9))
        frame.fill(0f)
        val top = probs.indices.filter {
            p.isSpecies[it] && probs[it] >= SoundSensing.SPECIES_MIN && network.hone.app.birds.Range.allowed(ctx, p.names[it], probs[it])
        }.sortedByDescending { probs[it] }.take(5)
        for (i in top) {
            val prev = win.species[p.names[i]]
            win.species[p.names[i]] = Pair(max(prev?.first ?: 0f, probs[i]), (prev?.second ?: 0) + 1)
        }
        if (top.isNotEmpty()) runCatching {
            val f = File(StationStore.dataDir(ctx, st.id), "detections.jsonl")
            f.appendText(top.joinToString("") { i ->
                JSONObject().put("t", now).put("name", p.names[i]).put("conf", Math.round(probs[i] * 1000) / 10.0).toString() + "\n"
            })
            for (i in top) network.hone.app.engage.Engagement.onDetection(ctx, p.names[i], now, probs[i])
        }
        Stations.update(st.id) { it.copy(frames = it.frames + 1, birds = it.birds + top.size, lastFrameMs = now) }
    }

    @Synchronized fun flush() { if (win.frames > 0) closeWindow(win); win = Win(System.currentTimeMillis() / WINDOW_MS) }

    private fun closeWindow(w: Win) {
        if (w.frames == 0) return
        val sorted = w.blockDb.sorted()
        fun pct(q: Double) = if (sorted.isEmpty()) null else Math.round(sorted[((1 - q) * (sorted.size - 1)).toInt()] * 10) / 10.0
        val r = JSONObject().put("v", 1).put("source", "station").put("station", st.id).put("tier", st.tier)
            .put("window", w.index).put("model", Perch.MODEL_ID).put("listen_seconds", w.frames * 5)
            .put("frames", w.frames).put("frames_discarded_human", w.human)
            .put("species", JSONArray().apply {
                w.species.entries.sortedByDescending { it.value.first }.forEach { (k, v) ->
                    put(JSONObject().put("name", k).put("conf_q8", (v.first * 255).toInt()).put("frames", v.second))
                }
            })
            .put("noise", JSONObject().put("unit", "dBFS").put("l90", pct(0.90) ?: JSONObject.NULL).put("l50", pct(0.50) ?: JSONObject.NULL))
        runCatching { File(StationStore.dataDir(ctx, st.id), "reports.jsonl").appendText(r.toString() + "\n") }
    }

    companion object { const val WINDOW_MS = 900_000L }
}

/** One camera: connect (audio track only), decode, feed the pipeline; reconnect with backoff. */
class StationListener(private val ctx: Context, private val st: Station) {
    @Volatile private var running = false
    private var thread: Thread? = null
    val isRunning get() = running

    fun start() {
        if (running) return
        running = true
        thread = Thread({ loop() }, "hone-station-${st.id.take(6)}").apply { isDaemon = true; start() }
    }

    fun stop() { running = false; thread?.interrupt(); thread = null; Stations.update(st.id) { it.copy(running = false) } }

    private fun loop() {
        var failures = 0
        while (running) {
            val creds = StationStore.creds(ctx, st.id)
            val c = RtspAudioClient(st.host, st.port, st.path, creds?.first?.ifEmpty { null }, creds?.second, timeoutMs = 10_000)
            var pipeline: StationPipeline? = null
            try {
                val m = c.open()
                val dec = AudioDecoders.forMedia(m) { AacDecoder(it) } ?: throw RtspException(415, "audio codec not supported: ${m.encoding}")
                Stations.update(st.id) { it.copy(running = true, reachable = true, lastError = null) }
                failures = 0
                pipeline = StationPipeline(ctx, st, dec.rate)
                val t0 = System.currentTimeMillis()
                var lastRate = t0
                dec.use {
                    c.stream({ !running }) { payload, _, _ ->
                        if (m.encoding == "MPEG4-GENERIC") {
                            for (u in Rtp.aacUnits(payload, m.fmtp["sizelength"]?.toIntOrNull() ?: 13, m.fmtp["indexlength"]?.toIntOrNull() ?: 3))
                                pipeline.feed(dec.decode(u))
                        } else pipeline.feed(dec.decode(payload))
                        val now = System.currentTimeMillis()
                        if (now - lastRate > 30_000) {
                            val kb = (c.bytesIn * 8 / ((now - t0) / 1000.0).coerceAtLeast(1.0) / 1000).toInt()
                            Stations.update(st.id) { it.copy(kbps = kb) }; lastRate = now
                        }
                    }
                }
            } catch (e: Exception) {
                failures++
                val msg = when { e is RtspException && e.code == 401 -> "login refused"; e is RtspException -> e.message
                    else -> "unreachable (${e.javaClass.simpleName})" }
                Stations.update(st.id) { it.copy(reachable = e is RtspException && e.code != 0, lastError = msg) }
                Log.w("HoneStation", "station ${st.id.take(6)}: $msg")
            } finally {
                pipeline?.flush()
                c.close()
            }
            if (!running) break
            try { Thread.sleep(RtspAudioClient.backoffMs(failures)) } catch (_: InterruptedException) { break }
        }
        Stations.update(st.id) { it.copy(running = false) }
    }
}

package network.hone.app.telemetry

import android.content.Context
import android.os.SystemClock
import android.util.Log
import network.hone.app.sightings.Cells
import network.hone.app.sightings.SightingsEngine
import org.json.JSONObject
import java.io.File

/**
 * Publishes Flipper frame commitments to the HONE indexer (not the chain: usefulness
 * scoring is off on chain). Each record is the SENSOR_DATA_COMMIT the Flipper signed
 * (sensor id, owner, batch hash, count, type, the Flipper's own signature and pubkey) plus
 * this phone's coarse stamp: minute and H3 res-7 cell. Batched every 15 min, at most
 * [BATCH] per POST and [MAX_POSTS] posts per round; the queue survives restarts.
 */
object FlipperPublisher {
    private const val TAG = "HoneFlipperIdx"
    const val URL = "${Telemetry.API}/flipper/frames"
    const val PERIOD_MS = 15 * 60_000L
    const val BATCH = 200
    private const val MAX_POSTS = 3
    private const val MAX_QUEUE = 5000

    @Volatile private var lastFlushMs = 0L

    private fun file(ctx: Context) = File(ctx.filesDir, "hone-node/flipper_index_queue.jsonl")

    /** Queue one verified Flipper commit (called by HoneAnnounceScanner after its signature checked out). */
    @Synchronized
    fun record(ctx: Context, sensorId: String, owner: String, batchHash: String, count: Int, sensorType: String,
               signatureHex: String, devicePubHex: String, rssi: Int?) {
        val fix = SightingsEngine.lastFix()
        val fresh = fix != null && (SystemClock.elapsedRealtimeNanos() - fix.elapsedRealtimeNanos) < 3_600_000_000_000L
        val cell = if (fresh) Cells.cell(fix!!.latitude, fix.longitude, 7) else null
        val rec = mapOf(
            "sensor_id" to sensorId, "owner" to owner, "batch_hash" to batchHash, "reading_count" to count,
            "sensor_type" to sensorType, "signature" to signatureHex, "device_pubkey" to devicePubHex,
            "t_min" to System.currentTimeMillis() / 60_000L, "cell" to cell, "rssi" to rssi,
        )
        val f = file(ctx).apply { parentFile?.mkdirs() }
        runCatching {
            if (f.exists() && f.length() > 0 && f.readLines().size >= MAX_QUEUE) {
                val keep = f.readLines().takeLast(MAX_QUEUE - 1)   // oldest out first
                f.writeText(keep.joinToString("\n", postfix = "\n"))
            }
            f.appendText(Canonical.encode(rec) + "\n")
        }.onFailure { Telemetry.error(ctx, "flipper queue", it) }
    }

    fun pending(ctx: Context): Int = runCatching { file(ctx).takeIf { it.exists() }?.readLines()?.count { it.isNotBlank() } ?: 0 }.getOrDefault(0)

    /** Called every service loop; posts when 15 min have passed since the last round. */
    fun maybeFlush(ctx: Context, force: Boolean = false) {
        val now = System.currentTimeMillis()
        if (!force && now - lastFlushMs < PERIOD_MS) return
        lastFlushMs = now
        val account = Telemetry.account(ctx) ?: run { Telemetry.skipped(ctx, "flipper", "no account"); return }
        repeat(MAX_POSTS) { if (!postOnce(ctx, account)) return }
    }

    /** One batch. Returns true when more may be sent this round. */
    @Synchronized
    private fun postOnce(ctx: Context, account: String): Boolean {
        val f = file(ctx)
        val lines = runCatching { f.readLines().filter { it.isNotBlank() } }.getOrDefault(emptyList())
        if (lines.isEmpty()) return false
        val batch = lines.take(BATCH)
        val frames = batch.mapNotNull { l -> runCatching { jsonToMap(JSONObject(l)) }.getOrNull() }
        val payload = mapOf("v" to 1, "kind" to "flipper_frames", "account" to account,
            "device" to Telemetry.deviceId(ctx), "ts" to System.currentTimeMillis() / 1000, "frames" to frames)
        val body = runCatching { Telemetry.signedBody(ctx, account, payload) }.getOrElse { Telemetry.error(ctx, "flipper sign", it); return false }
        val r = Telemetry.post(URL, body)
        if (r == null) { Telemetry.upload(ctx, "flipper", false, "offline"); return false }
        if (r.code == 429 || r.code >= 500) { Telemetry.upload(ctx, "flipper", false, "HTTP ${r.code}"); return false }
        // 2xx: stored. Other 4xx: this batch is malformed for good; drop it so the queue moves on.
        Telemetry.upload(ctx, "flipper", r.code in 200..299, if (r.code in 200..299) null else "HTTP ${r.code} ${r.body.take(120)}")
        runCatching { f.writeText(lines.drop(batch.size).joinToString("") { it + "\n" }) }
        Log.i(TAG, "flipper frames x${batch.size} -> ${r.code}")
        return lines.size > batch.size
    }

    private fun jsonToMap(j: JSONObject): Map<String, Any?> = j.keys().asSequence().associateWith { k ->
        val v = j.get(k); if (v == JSONObject.NULL) null else v
    }
}

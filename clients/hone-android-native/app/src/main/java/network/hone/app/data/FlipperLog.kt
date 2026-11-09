package network.hone.app.data

import android.content.Context
import android.os.SystemClock
import network.hone.app.sightings.Cells
import network.hone.app.sightings.SightingsEngine
import org.json.JSONObject
import java.io.File

/**
 * On-phone log of what a HONE Flipper reported, for the Flipper screen. One line per verified
 * batch commit: minute, group (subghz / wifi / tags / ir), reading count, a short batch-hash id and
 * the phone's private res-9 cell. The Flipper's batches never carry MACs, tag ids or signal
 * contents, so neither does this. Kept 7 days on the phone, never uploaded.
 * Also remembers each Flipper's last announce (battery, power, paused, last seen) across restarts.
 */
object FlipperLog {
    data class Entry(val t: Long, val dev: String, val group: String, val n: Int, val cell: String?, val id: String)
    data class Known(val id: String, val owner: String, val battery: Int, val extPower: Boolean, val paused: Boolean,
                     val caps: List<String>, val lastSeenMs: Long, val rssi: Int)

    private const val KEEP_MS = 7 * 86_400_000L
    private const val MAX_LINES = 20_000
    private var adds = 0
    private val savedAt = HashMap<String, Long>()
    private var cache: MutableMap<String, Known>? = null

    private fun logFile(ctx: Context) = File(ctx.filesDir, "hone-node/flipper_log.jsonl")
    private fun knownFile(ctx: Context) = File(ctx.filesDir, "hone-node/flipper_known.json")

    @Synchronized
    fun add(ctx: Context, dev: String, group: String, n: Int, batchHash: String) {
        val fix = SightingsEngine.lastFix()
        val fresh = fix != null && (SystemClock.elapsedRealtimeNanos() - fix.elapsedRealtimeNanos) < 3_600_000_000_000L
        val cell = if (fresh) Cells.cell(fix!!.latitude, fix.longitude, 9) else null
        val j = JSONObject().put("t", System.currentTimeMillis()).put("d", dev).put("g", group).put("n", n)
            .put("h", batchHash.take(12))
        if (cell != null) j.put("c", cell)
        runCatching {
            val f = logFile(ctx).apply { parentFile?.mkdirs() }
            f.appendText(j.toString() + "\n")
            if (++adds % 200 == 0) trim(f)
        }
    }

    private fun trim(f: File) {
        val cut = System.currentTimeMillis() - KEEP_MS
        val keep = f.readLines().filter { l -> runCatching { JSONObject(l).getLong("t") >= cut }.getOrDefault(false) }
            .takeLast(MAX_LINES)
        f.writeText(keep.joinToString("") { it + "\n" })
    }

    fun read(ctx: Context, sinceMs: Long): List<Entry> = runCatching {
        val f = logFile(ctx); if (!f.exists()) return emptyList()
        f.readLines().mapNotNull { l ->
            runCatching {
                val j = JSONObject(l); val t = j.getLong("t")
                if (t < sinceMs) null else Entry(t, j.optString("d"), j.optString("g"), j.optInt("n"),
                    j.optString("c").takeIf { it.isNotEmpty() }, j.optString("h"))
            }.getOrNull()
        }
    }.getOrDefault(emptyList())

    /** Remember an announce. Written at most once a minute per Flipper (or when battery/state changes). */
    @Synchronized
    fun seen(ctx: Context, d: HoneDevice) {
        if (!d.signatureOk) return
        val all = cache ?: known(ctx).associateBy { it.id }.toMutableMap().also { cache = it }
        val prev = all[d.deviceId]
        val changed = prev == null || prev.battery != d.batteryPct || prev.extPower != d.extPower || prev.paused != d.paused
        if (!changed && d.lastSeenMs - (savedAt[d.deviceId] ?: 0L) < 60_000) return
        savedAt[d.deviceId] = d.lastSeenMs
        all[d.deviceId] = Known(d.deviceId, d.owner, d.batteryPct, d.extPower, d.paused, d.caps, d.lastSeenMs, d.rssi)
        val arr = org.json.JSONArray()
        all.values.forEach { k ->
            arr.put(JSONObject().put("id", k.id).put("owner", k.owner).put("battery", k.battery).put("ext", k.extPower)
                .put("paused", k.paused).put("caps", org.json.JSONArray(k.caps)).put("seen", k.lastSeenMs).put("rssi", k.rssi))
        }
        runCatching { knownFile(ctx).apply { parentFile?.mkdirs() }.writeText(arr.toString()) }
    }

    fun known(ctx: Context): List<Known> = runCatching {
        val f = knownFile(ctx); if (!f.exists()) return emptyList()
        val a = org.json.JSONArray(f.readText())
        (0 until a.length()).map { i ->
            val o = a.getJSONObject(i); val caps = o.optJSONArray("caps")
            Known(o.getString("id"), o.optString("owner"), o.optInt("battery", -1), o.optBoolean("ext"), o.optBoolean("paused"),
                caps?.let { c -> (0 until c.length()).map { c.getString(it) } } ?: emptyList(), o.optLong("seen"), o.optInt("rssi"))
        }
    }.getOrDefault(emptyList())
}
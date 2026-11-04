package network.hone.app.telemetry

import android.content.Context
import android.util.Log
import network.hone.app.data.ChainHttp
import network.hone.app.sightings.Cells
import network.hone.app.sightings.IdKeys
import network.hone.app.sightings.SightingsEngine
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Coverage summaries (public_summary rows) to the chain as SENSOR_SUMMARY entries, signed by
 * this device's seat key (owner = "account@<seat pubkey hex>"), and to the indexer map.
 *  - Chain: only once the chain says usefulness is active (active_from_epoch <= tip); one entry
 *    per (res-9 cell, window) with all its metrics; rows older than 7 days are not sent to the
 *    chain (they earn 0 and still cost the fee); paced ~1 per 2 s, at most [PER_ROUND] per round.
 *  - Indexer map (/api/coverage/summaries): every located row, any age, batched every 15 min.
 * Duplicates and replays score 0 on chain, so an accepted row is never resent.
 */
object CoverageSubmitter {
    private const val TAG = "HoneCoverage"
    const val CHAIN_URL = "${ChainHttp.PUBLIC_BASE}/api/sensor/summary"
    const val INDEX_URL = "${Telemetry.API}/coverage/summaries"
    private const val ROUND_MS = 5 * 60_000L
    private const val INDEX_MS = 15 * 60_000L
    private const val PER_ROUND = 30
    private const val PACE_MS = 2_000L
    private const val MAX_AGE_WINDOWS = 7 * 96L

    // public_summary.submitted_at markers (queued=0 once handled for the chain)
    const val MAP_ONLY = -1L      // too old / too coarse / unlocated: never goes to the chain
    const val REFUSED = -2L       // chain refused it for good

    private val busy = AtomicBoolean(false)
    @Volatile private var lastRoundMs = 0L
    @Volatile private var lastIndexMs = 0L

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_coverage", Context.MODE_PRIVATE)

    /** Called every service loop; does its work on a background thread. */
    fun maybeFlush(ctx: Context) {
        val now = System.currentTimeMillis()
        val chainDue = now - lastRoundMs >= ROUND_MS
        val indexDue = now - lastIndexMs >= INDEX_MS
        if (!chainDue && !indexDue) return
        if (!busy.compareAndSet(false, true)) return
        val app = ctx.applicationContext
        Thread({
            try {
                if (indexDue) { lastIndexMs = now; runCatching { indexRound(app) }.onFailure { Telemetry.error(app, "coverage index", it) } }
                if (chainDue) { lastRoundMs = now; runCatching { chainRound(app) }.onFailure { Telemetry.error(app, "coverage chain", it) } }
            } finally { busy.set(false) }
        }, "hone-coverage").start()
    }

    /** Integer metrics from a v2 summary json (at most 32; names [a-z0-9._:]). */
    fun metrics(j: JSONObject): Map<String, Long> {
        val m = LinkedHashMap<String, Long>()
        fun put(k: String, v: Any?) { if (v is Number) m[k] = v.toLong() }
        j.optJSONObject("wifi")?.let { w ->
            put("wifi.ap_count", w.opt("ap_count")); put("wifi.scans", w.opt("scans"))
            put("wifi.rssi_p50", w.opt("rssi_p50")); put("wifi.rssi_max", w.opt("rssi_max"))
            w.optJSONObject("by_band")?.let { b -> listOf("2g", "5g", "6g").forEach { put("wifi.band.$it", b.opt(it)) } }
            w.optJSONArray("rssi_hist")?.let { h -> for (i in 0 until minOf(7, h.length())) put("wifi.rssi_hist.$i", h.opt(i)) }
        }
        j.optJSONObject("ble")?.let { b -> put("ble.crowd", b.opt("crowd")); put("ble.scan_s", b.opt("scan_s")) }
        j.optJSONObject("hone")?.let { put("hone.devices", it.opt("devices")) }
        put("mobile_ap", j.opt("mobile_ap"))
        j.optJSONObject("loc")?.let { l -> put("loc.samples", l.opt("samples")); put("loc.acc_m_p50", l.opt("acc_m_p50")) }
        return m.entries.take(SensorSummaryWire.MAX_METRICS).associate { it.key to it.value }
    }

    private fun seatId(ctx: Context): String? = Telemetry.seatAccount(ctx)?.let { "$it@${Telemetry.seatPub(ctx)}" }

    /** Returns null when submitting is allowed, else the reason it is not. */
    private fun gate(ctx: Context, owner: String): String? {
        val tip = ChainHttp.get("/api/node/info")?.let { runCatching { JSONObject(it).getLong("epoch") }.getOrNull() }
            ?: return "chain unreachable"
        val u = ChainHttp.get("/api/sensor/usefulness/$owner") ?: return "usefulness endpoint unreachable"
        val j = runCatching { JSONObject(u) }.getOrNull() ?: return "usefulness unreadable"
        prefs(ctx).edit().putString("usefulness", u.take(4000)).putLong("usefulness_ms", System.currentTimeMillis()).apply()
        val from = j.optLong("active_from_epoch", -1)
        if (from <= 0 || tip < from) return "usefulness not active (from ${if (from > 0) from else "?"}, tip $tip)"
        return null
    }

    private fun chainRound(ctx: Context) {
        val store = SightingsEngine.storeOrNull() ?: return
        val owner = seatId(ctx) ?: run { Telemetry.skipped(ctx, "summary", "no seat account"); return }
        val nowW = IdKeys.window()
        val rows = store.queuedSummaries(400)
        if (rows.isEmpty()) return
        // Rows that can never go to the chain are marked map-only first (no network needed).
        val eligible = LinkedHashMap<Pair<String, Long>, Triple<String, Long, JSONObject>>()  // (cell9, window) -> best row
        for ((cell, w, json) in rows) {
            val j = runCatching { JSONObject(json) }.getOrNull()
            val r = if (cell == "unlocated") -1 else Cells.res(cell)
            val c9 = if (r >= 9) Cells.parent(cell, 9) else null
            if (j == null || c9 == null || w < nowW - MAX_AGE_WINDOWS || w >= nowW) {
                if (w < nowW) store.markSummary(cell, w, MAP_ONLY)
                continue
            }
            val k = c9 to w
            val prev = eligible[k]
            val samples = j.optJSONObject("loc")?.optInt("samples") ?: 0
            if (prev == null || samples > (prev.third.optJSONObject("loc")?.optInt("samples") ?: 0)) {
                prev?.let { store.markSummary(it.first, it.second, MAP_ONLY) }   // one entry per res-9 cell/window
                eligible[k] = Triple(cell, w, j)
            } else store.markSummary(cell, w, MAP_ONLY)
        }
        if (eligible.isEmpty()) return
        gate(ctx, owner)?.let { Telemetry.skipped(ctx, "summary", it); return }
        val seed = Telemetry.seatSeedForChain(ctx)
        val sensorId = "coverage-${Telemetry.deviceId(ctx)}"
        var sent = 0
        for ((k, row) in eligible) {
            if (sent >= PER_ROUND) break
            val (c9, w) = k
            val met = metrics(row.third).filterKeys { SensorSummaryWire.METRIC.matches(it) }
            if (met.isEmpty()) { store.markSummary(row.first, row.second, MAP_ONLY); continue }
            val msg = SensorSummaryWire.message("hone", sensorId, owner, "", c9, w, met)
            val body = SensorSummaryWire.body(sensorId, owner, "", c9, w, met, Canonical.hex(Canonical.sign(seed, msg)))
            val r = Telemetry.post(CHAIN_URL, body)
            when {
                r == null -> { Telemetry.upload(ctx, "summary", false, "offline"); return }
                r.code == 429 || r.code >= 500 -> { Telemetry.upload(ctx, "summary", false, "HTTP ${r.code}"); return }
                else -> {
                    val j = runCatching { JSONObject(r.body) }.getOrNull()
                    if (r.code in 200..299 && j?.optBoolean("accepted") == true) {
                        store.markSummary(row.first, row.second, System.currentTimeMillis())
                        Telemetry.upload(ctx, "summary", true)
                    } else {
                        val err = j?.optString("error")?.ifEmpty { null } ?: "HTTP ${r.code} ${r.body.take(100)}"
                        Telemetry.upload(ctx, "summary", false, err)
                        if (err.contains("not active")) return               // chain side not live: keep rows queued
                        store.markSummary(row.first, row.second, REFUSED)
                    }
                }
            }
            sent++
            Thread.sleep(PACE_MS)
        }
        Log.i(TAG, "summary round: $sent sent")
    }

    /** Every located row (any age) to the indexer coverage map, signed like the other uploads. */
    private fun indexRound(ctx: Context) {
        val store = SightingsEngine.storeOrNull() ?: return
        val account = Telemetry.account(ctx) ?: return
        val after = prefs(ctx).getLong("index_rowid", 0)
        val rows = store.summariesAfter(after, 300)
        if (rows.isEmpty()) return
        val items = rows.mapNotNull { (rowid, cell, w, json) ->
            if (cell == "unlocated") null else {
                val j = runCatching { JSONObject(json) }.getOrNull() ?: return@mapNotNull null
                val c = if (Cells.res(cell) > 9) Cells.parent(cell, 9) else cell
                c?.let { mapOf("cell" to it, "window" to w, "metrics" to metrics(j)) }
            }
        }
        if (items.isNotEmpty()) {
            val payload = mapOf("v" to 1, "kind" to "coverage", "account" to account, "device" to Telemetry.deviceId(ctx),
                "ts" to System.currentTimeMillis() / 1000, "items" to items)
            val r = Telemetry.post(INDEX_URL, Telemetry.signedBody(ctx, account, payload))
            if (r == null || r.code == 429 || r.code >= 500) { Telemetry.upload(ctx, "coverage_map", false, r?.let { "HTTP ${it.code}" } ?: "offline"); return }
            Telemetry.upload(ctx, "coverage_map", r.code in 200..299, if (r.code in 200..299) null else "HTTP ${r.code} ${r.body.take(100)}")
        }
        prefs(ctx).edit().putLong("index_rowid", rows.last().rowid).apply()
    }

    /** Usefulness snapshot for the status (and the Today card later): points and claim counts. */
    fun usefulness(ctx: Context): Map<String, Any?>? {
        val s = prefs(ctx).getString("usefulness", null) ?: return null
        val j = runCatching { JSONObject(s) }.getOrNull() ?: return null
        val c = j.optJSONObject("counts")
        return mapOf("active_from_epoch" to j.optLong("active_from_epoch"), "proven_points" to j.optLong("proven_points"),
            "queued_points" to j.optLong("queued_points"),
            "pending" to c?.optLong("pending"), "proven" to c?.optLong("proven"), "void" to c?.optLong("void"),
            "age_s" to (System.currentTimeMillis() - prefs(ctx).getLong("usefulness_ms", 0)) / 1000)
    }
}

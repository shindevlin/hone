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
 *  - Chain: only once the chain says usefulness is active (active_from_epoch <= tip); EVERY row
 *    (Josh 2026-11-10 "send everything"): every row per (cell, window), any age (old rows keep their
 *    window and simply earn no claim), and unlocated rows at the device's own coarse area flagged
 *    loc.inferred=1 (loc.src, loc.age_s). Res 7 at/near home, on the home Wi-Fi or with no home
 *    known, else at most res 9. batch_hash = sha256 of the window's raw blob (RawRecorder).
 *    A row with no area at all waits (queued=3) for the chain's null-cell gate; nothing is dropped.
 *    Paced ~1 per 2 s, at most [PER_ROUND] per round.
 *  - Indexer map (/api/coverage/summaries): every located row, any age, batched every 15 min.
 * Duplicates and replays score 0 on chain, so an accepted row is never resent.
 */
object CoverageSubmitter {
    private const val TAG = "HoneCoverage"
    const val CHAIN_URL = "${ChainHttp.PUBLIC_BASE}/api/sensor/summary"

    /** True when [cell] is at or near the saved home (home radius plus a block's size). */
    private fun nearHome(ctx: Context, cell: String): Boolean {
        val h = network.hone.app.home.HomeState.home(ctx) ?: return false
        val c = Cells.center(cell) ?: return true
        val d = FloatArray(1)
        android.location.Location.distanceBetween(c.first, c.second, h.first, h.second, d)
        return d[0] <= h.third + Cells.EDGE_M[9].toFloat()
    }

    /** The cell this row goes on chain at (see SensorSummaryWire.chainRes), or null (map only). */
    fun chainCell(ctx: Context, cell: String, atHome: Boolean): String? {
        val r = Cells.res(cell)
        val homeKnown = network.hone.app.home.HomeState.home(ctx) != null
        val res = SensorSummaryWire.chainRes(r, homeKnown, homeKnown && nearHome(ctx, cell), atHome) ?: return null
        return if (r == res) cell else Cells.parent(cell, res)
    }

    /**
     * The chain accepts res 7-8 summary cells only once its gate is active. Read from the node:
     * coarse_cells_from_epoch (epoch, active when tip >= it) or summary_min_res (<= 7), in
     * /api/sensor/usefulness/<owner> or /api/node/info. Cached 10 min.
     */
    @Volatile private var coarseGate: Pair<Long, Boolean>? = null
    fun coarseCellsActive(owner: String): Boolean {
        coarseGate?.let { (t, v) -> if (System.currentTimeMillis() - t < 10 * 60_000L) return v }
        val info = ChainHttp.get("/api/node/info")?.let { runCatching { JSONObject(it) }.getOrNull() }
        val use = ChainHttp.get("/api/sensor/usefulness/$owner")?.let { runCatching { JSONObject(it) }.getOrNull() }
        if (info == null) return coarseGate?.second ?: false
        val v = coarseActive(info.optLong("epoch", -1),
            listOfNotNull(use, info).map { it.optLong("coarse_cells_from_epoch", -1) to it.optInt("summary_min_res", 99) })
        coarseGate = System.currentTimeMillis() to v
        return v
    }

    /** Pure: is the res 7-8 gate open, given the tip and each doc's (coarse_cells_from_epoch, summary_min_res). */
    fun coarseActive(tip: Long, gates: List<Pair<Long, Int>>): Boolean =
        gates.any { (from, minRes) -> (from in 1..tip) || minRes <= SensorSummaryWire.MIN_RES }
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
        j.optJSONObject("flipper")?.let { f -> listOf("subghz", "wifi", "tags", "ir").forEach { put("flipper.$it", f.opt(it)) } }
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
        // Every located row goes on chain; only its place is coarsened (res 7 at/near home, on the
        // home Wi-Fi or with no home known; elsewhere at most res 9). Res-7 rows wait (held, never
        // dropped) until the chain's res 7-9 gate is open.
        val p = prefs(ctx)
        if (!p.getBoolean("requeue_v24", false)) {
            val n = runCatching { store.requeueMapOnly() }.getOrDefault(0)
            p.edit().putBoolean("requeue_v24", true).apply(); Log.i(TAG, "requeued $n map-only rows")
        }
        if (!p.getBoolean("requeue_v27", false)) {
            val n = runCatching { store.requeueSendAll() }.getOrDefault(0)
            p.edit().putBoolean("requeue_v27", true).apply(); Log.i(TAG, "send everything: requeued $n map-only/refused rows")
        }
        val coarseOk = coarseCellsActive(owner)
        if (coarseOk) runCatching { store.releaseHeld() }
        var held = 0
        var parked = 0
        val eligible = ArrayList<Pair<Pair<String, Long>, Triple<String, Long, JSONObject>>>()  // ((chain cell, window), row)
        for ((cell, w, json) in rows) {
            if (w >= nowW) continue                                     // window still open
            val j = runCatching { JSONObject(json) }.getOrNull() ?: JSONObject()
            // Unlocated rows go at the device's own coarse area (AreaLocator), flagged inferred.
            val base = if (cell == "unlocated") j.optString("area_cell").takeIf { SensorSummaryWire.CELL.matches(it) } else cell
            if (base == null) { store.parkSummary(cell, w); parked++; continue }     // no area at all: null-cell gate
            val target = chainCell(ctx, base, j.optBoolean("at_home")) ?: run { store.parkSummary(cell, w); parked++; null } ?: continue
            val coarse = SensorSummaryWire.res(target) < SensorSummaryWire.MAX_RES
            if (coarse && !coarseOk) { store.holdSummary(cell, w); held++; continue }
            eligible.add((target to w) to Triple(cell, w, j))                     // every row, any age
        }
        if (held > 0) Telemetry.skipped(ctx, "summary_res7", "$held held until the chain accepts res 7 cells")
        if (parked > 0) Telemetry.skipped(ctx, "summary_nocell", "$parked with no area at all wait for the chain's null-cell gate")
        if (eligible.isEmpty()) return
        gate(ctx, owner)?.let { Telemetry.skipped(ctx, "summary", it); return }
        val seed = Telemetry.seatSeedForChain(ctx)
        val sensorId = "coverage-${Telemetry.deviceId(ctx)}"
        var sent = 0
        for ((k, row) in eligible) {
            if (sent >= PER_ROUND) break
            val (c9, w) = k
            val met = LinkedHashMap(metrics(row.third).filterKeys { SensorSummaryWire.METRIC.matches(it) }.entries.take(SensorSummaryWire.MAX_METRICS - 4)
                .associate { it.key to it.value })
            if (row.third.optBoolean("loc_inferred")) {
                met["loc.inferred"] = 1L
                met["loc.src"] = locSrcCode(row.third.optString("loc_src"))
                if (row.third.has("loc_age_s")) met["loc.age_s"] = row.third.optLong("loc_age_s")
            }
            if (met.isEmpty()) met["obs.none"] = 1L                       // nothing observed is a reading too
            val bh = runCatching { network.hone.app.raw.RawRecorder.shaFor(ctx, w) }.getOrDefault("")
            val msg = SensorSummaryWire.message("hone", sensorId, owner, bh, c9, w, met)
            val body = SensorSummaryWire.body(sensorId, owner, bh, c9, w, met, Canonical.hex(Canonical.sign(seed, msg)))
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

    /** loc.src metric: 1 gps/fused, 2 network, 3 wifi_fp, 4 last verified, 5 ip, 6 home Wi-Fi, 0 none. */
    fun locSrcCode(s: String): Long = when (s) {
        "gps", "fused" -> 1; "network" -> 2; "wifi_fp" -> 3; "last_verified" -> 4; "ip" -> 5; "home_wifi" -> 6; else -> 0 }

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

package network.hone.app.birds

import android.content.Context
import android.util.Log
import network.hone.app.data.ChainHttp
import network.hone.app.sightings.IdKeys
import network.hone.app.telemetry.Canonical
import network.hone.app.telemetry.CoverageSubmitter
import network.hone.app.telemetry.SensorSummaryWire
import network.hone.app.telemetry.Telemetry
import org.json.JSONObject
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Bird sightings to the chain (hard-birds §3; Josh 13 Nov: "1 okay").
 *
 * The indexer decides what may earn: POST /api/birds/chain_rows (signed) returns this account's rows that are
 * public, rare-pending or upgraded (never held), 30 min after their window, as chain metrics
 * (bird.sp:/ge:/gr:/up:, value 2 = rare here). They go on chain as SENSOR_SUMMARY entries signed by the seat
 * key, at the same coarsened cell as coverage rows. Nothing is fetched or sent until the chain's bird gate is
 * open (bird_earn_from_epoch <= tip in /api/sensor/usefulness/<owner>): before it, bird.* names would be
 * scored like any other metric.
 */
object BirdChainSubmitter {
    private const val TAG = "HoneBirdChain"
    private const val ROUND_MS = 15 * 60_000L
    private const val PACE_MS = 2_000L
    private const val MAX_ENTRIES = 40
    private const val CHAIN_URL = CoverageSubmitter.CHAIN_URL
    private const val ROWS_URL = "${BirdsSync.BASE}/chain_rows"
    /** Rows older than this earn no claim on chain; the first round starts here. */
    private const val START_BACK_WINDOWS = 7 * 96L

    private val busy = AtomicBoolean(false)
    @Volatile private var lastRoundMs = 0L

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_bird_chain", Context.MODE_PRIVATE)

    /** One summary entry: chain cell, window and its bird metrics. */
    data class Entry(val cell: String, val window: Long, val metrics: Map<String, Long>)

    /**
     * Pure: indexer rows -> summary entries. [chainCellOf] maps a row's cell to its chain cell (null: map only).
     * Per (cell, window) the highest value of each metric wins; a species that is also an upgrade there goes only
     * as the upgrade (the group claim + upgrade already make one species); at most 32 metrics per entry.
     */
    fun entries(rows: List<Triple<String, Long, Pair<String, Long>>>, chainCellOf: (String) -> String?): List<Entry> {
        val by = java.util.TreeMap<Pair<String, Long>, java.util.TreeMap<String, Long>>(compareBy<Pair<String, Long>>({ it.second }, { it.first }))
        for ((cell, w, mv) in rows) {
            val (metric, value) = mv
            if (!SensorSummaryWire.METRIC.matches(metric) || !metric.startsWith("bird.") || value !in 1L..2L) continue
            val c = chainCellOf(cell) ?: continue
            val m = by.getOrPut(c to w) { java.util.TreeMap() }
            m[metric] = maxOf(m[metric] ?: 0L, value)
        }
        val out = ArrayList<Entry>()
        for ((k, m) in by) {
            m.keys.filter { it.startsWith("bird.up:") }.forEach { m.remove("bird.sp:" + it.removePrefix("bird.up:")) }
            m.entries.chunked(SensorSummaryWire.MAX_METRICS).forEach { ch -> out.add(Entry(k.first, k.second, ch.associate { it.key to it.value })) }
        }
        return out
    }

    /** Pure: is the chain's bird gate open? */
    fun gateOpen(tip: Long, birdFrom: Long): Boolean = birdFrom in 1..tip

    /** Called every service loop; does its work on a background thread at most every 15 min. */
    fun maybeFlush(ctx: Context) {
        val now = System.currentTimeMillis()
        if (now - lastRoundMs < ROUND_MS) return
        if (!busy.compareAndSet(false, true)) return
        lastRoundMs = now
        val app = ctx.applicationContext
        Thread({
            try { runCatching { round(app) }.onFailure { Telemetry.error(app, "bird chain", it) } } finally { busy.set(false) }
        }, "hone-bird-chain").start()
    }

    private fun round(ctx: Context) {
        val account = Telemetry.seatAccount(ctx) ?: return
        val owner = "$account@${Telemetry.seatPub(ctx)}"
        val tip = ChainHttp.get("/api/node/info")?.let { runCatching { JSONObject(it).getLong("epoch") }.getOrNull() } ?: return
        val use = ChainHttp.get("/api/sensor/usefulness/$owner")?.let { runCatching { JSONObject(it) }.getOrNull() } ?: return
        if (!gateOpen(tip, use.optLong("bird_earn_from_epoch", -1))) return     // not active (or an old node): send nothing
        val p = prefs(ctx)
        var after = p.getLong("after", -1)
        if (after < 0) after = IdKeys.window() - START_BACK_WINDOWS
        val payload = mapOf("v" to 1, "kind" to "chain_rows", "account" to account, "after" to after,
            "ts" to System.currentTimeMillis() / 1000)
        val r = Telemetry.post(ROWS_URL, Telemetry.signedBody(ctx, account, payload)) ?: return
        if (r.code != 200) { Telemetry.upload(ctx, "bird_chain_rows", false, "HTTP ${r.code} ${r.body.take(100)}"); return }
        val j = JSONObject(r.body)
        val arr = j.optJSONArray("rows")
        val rows = (0 until (arr?.length() ?: 0)).map { i ->
            val o = arr!!.getJSONObject(i)
            Triple(o.getString("cell"), o.getLong("window"), o.getString("metric") to o.getLong("value"))
        }
        val ents = entries(rows) { cell -> CoverageSubmitter.chainCell(ctx, cell, false) }
        if (ents.size > MAX_ENTRIES) Log.i(TAG, "${ents.size} entries; this round sends all of this page")
        val seed = Telemetry.seatSeedForChain(ctx)
        val sensorId = "birds-${Telemetry.deviceId(ctx)}"
        for (e in ents) {
            val msg = SensorSummaryWire.message("hone", sensorId, owner, "", e.cell, e.window, e.metrics)
            val body = SensorSummaryWire.body(sensorId, owner, "", e.cell, e.window, e.metrics, Canonical.hex(Canonical.sign(seed, msg)))
            val res = Telemetry.post(CHAIN_URL, body)
            if (res == null || res.code == 429 || res.code >= 500) {         // retry this page next round
                Telemetry.upload(ctx, "bird_chain", false, res?.let { "HTTP ${it.code}" } ?: "offline"); return
            }
            val ok = res.code in 200..299 && runCatching { JSONObject(res.body).optBoolean("accepted") }.getOrDefault(false)
            Telemetry.upload(ctx, "bird_chain", ok, if (ok) null else "HTTP ${res.code} ${res.body.take(100)}")
            if (!ok && res.body.contains("not active")) return
            Thread.sleep(PACE_MS)
        }
        p.edit().putLong("after", maxOf(after, j.optLong("upto", after))).apply()
        Log.i(TAG, "bird chain round: ${ents.size} entries up to window ${j.optLong("upto")}")
    }
}

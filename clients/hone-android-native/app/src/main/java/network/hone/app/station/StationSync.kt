package network.hone.app.station

import android.content.Context
import network.hone.app.birds.BirdsSync
import network.hone.app.data.ChainHttp
import network.hone.app.telemetry.Canonical
import network.hone.app.telemetry.Telemetry
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.util.UUID

/**
 * Station detections to the birds indexer (source=station, tier, hashed station id), every
 * 15 min, same publish rule as the phone: confident (>= 70 %) or heard twice in the window.
 * Chain rewards for stations wait for the chain's seat registration: [StationChain] only
 * checks the gate; nothing is submitted to the chain from here yet.
 */
object StationSync {
    private const val WINDOW_MS = 900_000L
    private const val MIN_CONF = 70.0
    @Volatile private var lastMs = 0L

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_stations", Context.MODE_PRIVATE)

    @Synchronized
    fun maybeFlush(ctx: Context, force: Boolean = false) {
        val now = System.currentTimeMillis()
        if (!force && now - lastMs < 15 * 60_000L) return
        lastMs = now
        val account = Telemetry.account(ctx) ?: run { Telemetry.skipped(ctx, "stations", "no account"); return }
        for (st in StationStore.list(ctx)) runCatching { push(ctx, st, account) }
            .onFailure { Telemetry.upload(ctx, "stations", false, it.message) }
        runCatching { StationChain.check(ctx) }
    }

    private fun push(ctx: Context, st: Station, account: String) {
        val cell = st.cell ?: run { Telemetry.skipped(ctx, "stations", "station has no location yet"); return }
        val f = File(StationStore.dataDir(ctx, st.id), "detections.jsonl")
        if (!f.exists()) return
        val current = System.currentTimeMillis() / WINDOW_MS
        val key = "sent_${st.id}"
        val last = prefs(ctx).getLong(key, st.addedMs / WINDOW_MS - 1)
        if (last >= current - 1) return
        val byWin = sortedMapOf<Long, HashMap<String, Pair<Double, Int>>>()
        f.forEachLine { l ->
            val j = runCatching { JSONObject(l) }.getOrNull() ?: return@forEachLine
            val w = j.getLong("t") / WINDOW_MS
            if (w <= last || w >= current) return@forEachLine
            val m = byWin.getOrPut(w) { HashMap() }
            val prev = m[j.getString("name")]
            m[j.getString("name")] = Pair(maxOf(prev?.first ?: 0.0, j.getDouble("conf")), (prev?.second ?: 0) + 1)
        }
        val show = ctx.getSharedPreferences("hone_birds", Context.MODE_PRIVATE).getBoolean("show_name", false)
        for ((w, spAll) in byWin) {
            val sp = spAll.filterValues { (c, n) -> c >= MIN_CONF || n >= 2 }
            if (sp.isNotEmpty()) {
                val payload = JSONObject().put("v", 1).put("sid", UUID.randomUUID().toString()).put("window", w).put("cell", cell)
                    .put("model", network.hone.app.sound.Perch.MODEL_ID).put("source", "station").put("tier", st.tier).put("station", st.id)
                    .put("species", JSONArray().apply {
                        sp.forEach { (n, v) -> put(JSONObject().put("name", n).put("conf_q8", (v.first * 2.55).toInt().coerceIn(0, 255)).put("frames", v.second)) }
                    })
                    .put("account", account).put("show_name", show)
                val s = Telemetry.signer(ctx, account)
                val body = Canonical.encode(Canonical.envelope(payload.toString(), s.keyType, s.seed))
                val r = Telemetry.post("${BirdsSync.BASE}/sightings", body) ?: run { Telemetry.upload(ctx, "stations", false, "offline"); return }
                if (r.code == 429 || r.code >= 500) { Telemetry.upload(ctx, "stations", false, "HTTP ${r.code}"); return }
                Telemetry.upload(ctx, "stations", r.code in 200..299, if (r.code in 200..299) null else "HTTP ${r.code} ${r.body.take(100)}")
                if (r.code in 200..299) runCatching {
                    val j = JSONObject(r.body)
                    j.optJSONArray("first_in_area")?.let { a -> for (i in 0 until a.length()) network.hone.app.engage.Engagement.firstInArea(ctx, a.getString(i)) }
                }
            }
            prefs(ctx).edit().putLong(key, w).apply()
        }
        prefs(ctx).edit().putLong(key, current - 1).apply()
    }
}

/**
 * Gate for station rewards on chain (chain side not designed yet): open only when the chain's
 * usefulness record for this device reports seat_registered and an active epoch. Until then
 * station data goes to the indexer and map only.
 */
object StationChain {
    fun check(ctx: Context) {
        val p = ctx.getSharedPreferences("hone_stations", Context.MODE_PRIVATE)
        if (System.currentTimeMillis() - p.getLong("gate_ms", 0) < 30 * 60_000L) return
        val owner = network.hone.app.account.AccountStore.referrer(ctx)?.let { "$it@${Telemetry.seatPub(ctx)}" }
        val state = if (owner == null) "no referrer" else {
            val tip = ChainHttp.get("/api/node/info")?.let { runCatching { JSONObject(it).getLong("epoch") }.getOrNull() }
            val u = ChainHttp.get("/api/sensor/usefulness/$owner")?.let { runCatching { JSONObject(it) }.getOrNull() }
            when {
                tip == null || u == null -> "chain unreachable"
                !u.optBoolean("seat_registered", false) -> "seat not registered"
                u.optLong("active_from_epoch", -1).let { it <= 0 || tip < it } -> "not active"
                else -> "active (submission not built yet)"
            }
        }
        p.edit().putString("gate", state).putLong("gate_ms", System.currentTimeMillis()).apply()
    }

    fun state(ctx: Context): String? = ctx.getSharedPreferences("hone_stations", Context.MODE_PRIVATE).getString("gate", null)
}

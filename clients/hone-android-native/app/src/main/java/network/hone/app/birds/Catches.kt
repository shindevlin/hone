package network.hone.app.birds

import android.content.Context
import network.hone.app.sightings.Cells
import network.hone.app.sightings.SightingsEngine
import network.hone.app.sound.Detection
import org.json.JSONObject
import java.io.File
import java.net.URLEncoder
import kotlin.math.exp
import kotlin.math.ln
import kotlin.math.max
import kotlin.math.min

/**
 * Ornigo catches. A catch is evidence, not a single score: each call adds log-odds the same way
 * the indexer's occupancy model does (EVENT_BASE + logit(conf) / T, with the species' own
 * temperature T), on top of a prior for this area (in range this month, Regular here, or unlikely
 * here). The bird is caught once at least three calls are in and the evidence reaches CATCH_P.
 * Every encounter's outcome is logged on the phone (counts and scores only, never audio).
 */
object Catches {
    /** Weakest call the encounter listens to (field data: most real calls score 30-60%). */
    const val CALL_FLOOR = 0.30f
    const val MIN_CALLS = 3
    const val WINDOW_MS = 120_000L
    private const val CATCH_P = 0.90          // same bar as "likely" on the map
    private const val MIN_EVIDENCE = 1.5      // even a Regular-here bird needs some real calls
    private const val EVENT_BASE = 0.8
    private const val DEFAULT_T = 1.5
    /** Local fallbacks when the indexer can't be reached (same priors it uses). */
    private val PRIOR_P = mapOf("in_season" to 0.20, "off_season" to 0.03, "unknown" to 0.05, "out" to 0.003)
    /** Cosine similarity above which two calls are the same sound played again (a recording). */
    private const val REPLAY_SIM = 0.985f

    private fun dir(ctx: Context) = File(ctx.filesDir, "hone-node/sound").apply { mkdirs() }
    private fun file(ctx: Context) = File(dir(ctx), "catches.jsonl")
    private fun log(ctx: Context) = File(dir(ctx), "encounters.jsonl")

    fun record(ctx: Context, latin: String, conf: Float, unlikely: Boolean) {
        runCatching {
            file(ctx).appendText(JSONObject().put("t", System.currentTimeMillis()).put("name", latin)
                .put("conf", conf.toDouble()).put("unlikely", unlikely).toString() + "\n")
        }
    }

    data class Catch(val t: Long, val name: String)

    fun all(ctx: Context): List<Catch> {
        val f = file(ctx); if (!f.exists()) return emptyList()
        return f.readLines().mapNotNull { l -> runCatching { JSONObject(l).let { Catch(it.getLong("t"), it.getString("name")) } }.getOrNull() }
    }

    fun count(ctx: Context, latin: String) = all(ctx).count { it.name == latin }

    // ── prior ────────────────────────────────────────────────────────────────────
    data class Prior(val kind: String, val priorP: Double, val t: Double, val areaP: Double, val status: String?, val source: String)

    private val priors = HashMap<String, Pair<Long, Prior>>()

    /** Prior for [latin] here: from the indexer (per-species T, season, occupancy), else local range. Call off the main thread. */
    fun prior(ctx: Context, latin: String): Prior {
        // Where the phone is; without a fresh fix, the home area saved at first run.
        val fix = network.hone.app.home.HomeState.fixOrHome(ctx)
        val hp = ctx.getSharedPreferences("hone_birds", Context.MODE_PRIVATE)
        val last = fix ?: runCatching {
            val lm = ctx.getSystemService(android.location.LocationManager::class.java)
            listOf("fused", "gps", "network").firstNotNullOfOrNull { p -> runCatching { lm?.getLastKnownLocation(p) }.getOrNull() }
        }.getOrNull()
        val c7 = last?.let { Cells.cell(it.latitude, it.longitude, 7) }
            ?: if (hp.contains("home_lat")) Cells.cell(hp.getFloat("home_lat", 0f).toDouble(), hp.getFloat("home_lon", 0f).toDouble(), 7) else null
        val key = "$c7|$latin"
        priors[key]?.let { (at, p) -> if (System.currentTimeMillis() - at < 6 * 3600_000L) return p }
        val p = runCatching {
            if (c7 == null) error("no fix")
            val j = JSONObject(BirdsSync.getJson("/catch_prior?cell=$c7&latin=" + URLEncoder.encode(latin, "UTF-8"), 8_000))
            var kind = j.getString("kind"); var pp = j.getDouble("prior_p")
            // The indexer may still be fetching this area's range; the phone's own list can answer.
            if (kind == "unknown") Range.current(ctx)?.let { r ->
                kind = if (latin in r) "in_season" else "out"; pp = PRIOR_P.getValue(kind)
            }
            Prior(kind, pp, j.optDouble("T", DEFAULT_T), j.optDouble("area_p", 0.0),
                if (j.isNull("status")) null else j.optString("status"), "indexer")
        }.getOrElse { e ->
            android.util.Log.w("HoneCatch", "prior from indexer failed: ${e.message}")
            val kind = when (Range.current(ctx)?.contains(latin)) { true -> "in_season"; false -> "out"; null -> "unknown" }
            Prior(kind, PRIOR_P.getValue(kind), DEFAULT_T, 0.0, null, "local")
        }
        priors[key] = System.currentTimeMillis() to p
        android.util.Log.i("HoneCatch", "prior $latin: ${p.kind} ${p.priorP} area ${p.areaP} (${p.source})")
        return p
    }

    // ── score ────────────────────────────────────────────────────────────────────
    data class Score(val calls: Int, val evidence: Double, val need: Double, val p: Double, val fill: Float, val caught: Boolean)

    private fun logit(p: Double): Double { val q = min(max(p, 1e-6), 1 - 1e-6); return ln(q / (1 - q)) }
    private fun sigmoid(x: Double) = 1 / (1 + exp(-x))

    fun score(calls: List<Detection>, pr: Prior): Score {
        // Regular here lifts the prior; unlikely here keeps the low out-of-range prior whatever the area says.
        val l0 = logit(if (pr.kind == "out") pr.priorP else max(pr.priorP, pr.areaP))
        val ev = calls.sumOf { EVENT_BASE + logit(it.conf.toDouble()) / pr.t }
        val need = max(logit(CATCH_P) - l0, MIN_EVIDENCE)
        val fill = min(calls.size / MIN_CALLS.toFloat(), (ev / need).toFloat()).coerceIn(0f, 1f)
        return Score(calls.size, ev, need, sigmoid(l0 + ev), fill, calls.size >= MIN_CALLS && ev >= need)
    }

    /**
     * Replay check. Live birds never repeat a call sample for sample, so their Perch embeddings
     * differ a little every time; a recording played on a loop gives near-identical embeddings.
     */
    fun looksReplayed(calls: List<Detection>): Boolean {
        val e = calls.mapNotNull { it.emb }
        for (i in e.indices) for (j in i + 1 until e.size) if (cos(e[i], e[j]) > REPLAY_SIM) return true
        return false
    }

    private fun cos(a: FloatArray, b: FloatArray): Float {
        var d = 0.0; var na = 0.0; var nb = 0.0
        for (i in a.indices) { d += a[i] * b[i]; na += a[i] * a[i]; nb += b[i] * b[i] }
        return if (na == 0.0 || nb == 0.0) 0f else (d / Math.sqrt(na * nb)).toFloat()
    }

    // ── encounter log (for tuning; counts go into device_status) ──────────────────
    fun logOutcome(ctx: Context, startedAt: Long, target: String?, species: String?, outcome: String,
                   seen: Int, s: Score?, pr: Prior?, confs: List<Float>) {
        runCatching {
            log(ctx).appendText(JSONObject().put("t0", startedAt).put("t1", System.currentTimeMillis())
                .put("target", target).put("species", species).put("outcome", outcome).put("calls_seen", seen)
                .put("calls", s?.calls ?: 0).put("evidence", s?.evidence?.let { Math.round(it * 100) / 100.0 })
                .put("need", s?.need?.let { Math.round(it * 100) / 100.0 }).put("p", s?.p?.let { Math.round(it * 1000) / 1000.0 })
                .put("kind", pr?.kind).put("area_p", pr?.areaP).put("T", pr?.t).put("prior_src", pr?.source)
                .put("confs", org.json.JSONArray(confs.map { Math.round(it * 100) / 100.0 })).toString() + "\n")
        }
    }

    /** Encounter outcome counts for device_status: all time and the last 7 days. */
    fun stats(ctx: Context): Map<String, Any> {
        val f = log(ctx); if (!f.exists()) return mapOf("total" to 0)
        val since = System.currentTimeMillis() - 7 * 86_400_000L
        val all = HashMap<String, Int>(); val week = HashMap<String, Int>()
        var evSum = 0.0; var evN = 0
        f.forEachLine { l ->
            runCatching {
                val j = JSONObject(l); val o = j.getString("outcome")
                all[o] = (all[o] ?: 0) + 1
                if (j.getLong("t0") >= since) week[o] = (week[o] ?: 0) + 1
                if (!j.isNull("evidence")) { evSum += j.getDouble("evidence"); evN++ }
            }
        }
        return mapOf("total" to all.values.sum(), "outcomes" to all, "last_7d" to week,
            "mean_evidence" to if (evN > 0) Math.round(evSum / evN * 100) / 100.0 else null).filterValues { it != null }.mapValues { it.value!! }
    }
}

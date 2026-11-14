package network.hone.app.raw

import android.content.Context
import android.location.Location
import java.io.File
import java.security.MessageDigest

/**
 * Light GPS track and travel groups (Josh 2026-11-14).
 *
 * Track: while HONE runs, fixes are kept on this device only (hone-node/track.jsonl, 30 days),
 * thinned adaptively: about one a minute when moving, one every 10 minutes when still, plus
 * any move of 25 m. It is used only to place the owner's own readings; it is never uploaded.
 *
 * Travel group: the owner's devices travel together (a Flipper with both phones by default).
 * A Flipper reading that arrives without a live link gets the phone's position interpolated at
 * the reading's time, accepted only with a co-location proof: the two devices' radio
 * environments match ([colocate]) and the next sighting is home ([homeAnchor]). Match:
 * loc.src = travel_group with the proof hash; no match: tier C (inferred).
 */
object TravelGroup {
    data class Fix(val tMs: Long, val lat: Double, val lon: Double, val accM: Float, val speed: Float)
    data class Env(val wifi: Set<String> = emptySet(), val ble: Int? = null, val subghz: List<Long> = emptyList())
    data class Match(val score: Double, val match: Boolean, val parts: Map<String, Double>, val proof: String?)
    data class Placed(val tier: String, val src: String, val lat: Double?, val lon: Double?, val proof: String?, val score: Double?)

    // ── pure parts (unit-tested) ────────────────────────────────────────────────────────
    /** Keep [f] after [last]? ~1/min when moving, 1 per 10 min when still, or a 25 m move. */
    fun keep(last: Fix?, f: Fix): Boolean {
        if (last == null) return true
        val dt = f.tMs - last.tMs
        if (dt <= 0) return false
        val d = metres(last.lat, last.lon, f.lat, f.lon)
        val moving = f.speed > 0.8f || d > 25.0
        return d >= 25.0 || (moving && dt >= 60_000) || dt >= 600_000
    }

    /** Position at [tMs]: linear between the two fixes around it (both within 15 min), else the nearest within 5 min. */
    fun positionAt(track: List<Fix>, tMs: Long): Pair<Double, Double>? {
        if (track.isEmpty()) return null
        var lo = 0; var hi = track.size - 1
        if (tMs <= track[0].tMs) return track[0].takeIf { it.tMs - tMs <= 300_000 }?.let { it.lat to it.lon }
        if (tMs >= track[hi].tMs) return track[hi].takeIf { tMs - it.tMs <= 300_000 }?.let { it.lat to it.lon }
        while (hi - lo > 1) { val m = (lo + hi) / 2; if (track[m].tMs <= tMs) lo = m else hi = m }
        val a = track[lo]; val b = track[hi]
        if (tMs - a.tMs <= 900_000 && b.tMs - tMs <= 900_000) {
            val f = (tMs - a.tMs).toDouble() / (b.tMs - a.tMs).coerceAtLeast(1)
            return (a.lat + (b.lat - a.lat) * f) to (a.lon + (b.lon - a.lon) * f)
        }
        val n = if (tMs - a.tMs <= b.tMs - tMs) a else b
        return n.takeIf { Math.abs(n.tMs - tMs) <= 300_000 }?.let { it.lat to it.lon }
    }

    /** Same scorer as hone_rawfmt.colocate: Wi-Fi Jaccard 0.6, BLE density 0.2, sub-GHz peaks within 30 s 0.2. */
    fun colocate(a: Env, b: Env, threshold: Double = 0.35): Match {
        val parts = LinkedHashMap<String, Double>(); val w = LinkedHashMap<String, Double>()
        if (a.wifi.isNotEmpty() && b.wifi.isNotEmpty()) {
            parts["wifi"] = r4((a.wifi intersect b.wifi).size.toDouble() / (a.wifi union b.wifi).size); w["wifi"] = 0.6
        }
        if (a.ble != null && b.ble != null && (a.ble > 0 || b.ble > 0)) {
            parts["ble"] = r4(1 - Math.abs(a.ble - b.ble).toDouble() / maxOf(a.ble, b.ble, 1)); w["ble"] = 0.2
        }
        if (a.subghz.isNotEmpty() && b.subghz.isNotEmpty()) {
            parts["subghz"] = r4(a.subghz.count { x -> b.subghz.any { Math.abs(x - it) <= 30_000 } }.toDouble() / a.subghz.size); w["subghz"] = 0.2
        }
        if (w.isEmpty()) return Match(0.0, false, parts, null)
        val score = r4(w.entries.sumOf { (k, v) -> parts[k]!! * v } / w.values.sum())
        val match = score >= threshold && (parts["wifi"]?.let { it >= 0.2 } ?: true)
        val ev = RawFmt.json(mapOf("a" to mapOf("wifi" to a.wifi.sorted(), "ble" to a.ble, "subghz" to a.subghz),
            "b" to mapOf("wifi" to b.wifi.sorted(), "ble" to b.ble, "subghz" to b.subghz), "parts" to parts, "score" to score, "threshold" to threshold))
        return Match(score, match, parts, sha256(ev))
    }

    fun homeAnchor(next: Env, home: Env, threshold: Double = 0.3): Boolean =
        next.wifi.isNotEmpty() && home.wifi.isNotEmpty() &&
            (next.wifi intersect home.wifi).size.toDouble() / (next.wifi union home.wifi).size >= threshold

    /**
     * Place a reading of a travel-group device taken at [tMs] while its link was down.
     * Tier B (loc.src = travel_group) needs a track position, a co-location match and a home
     * anchor; otherwise tier C (inferred, no proof).
     */
    fun place(track: List<Fix>, tMs: Long, phoneEnv: Env, deviceEnv: Env, nextEnv: Env?, homeEnv: Env?): Placed {
        val pos = positionAt(track, tMs)
        val m = colocate(phoneEnv, deviceEnv)
        val anchored = nextEnv != null && homeEnv != null && homeAnchor(nextEnv, homeEnv)
        return if (pos != null && m.match && anchored) Placed("B", "travel_group", pos.first, pos.second, m.proof, m.score)
        else Placed("C", "inferred", pos?.first, pos?.second, null, m.score)
    }

    private fun r4(d: Double) = Math.round(d * 10_000) / 10_000.0
    fun metres(a1: Double, o1: Double, a2: Double, o2: Double): Double {
        val r = 6_371_000.0; val dLat = Math.toRadians(a2 - a1); val dLon = Math.toRadians(o2 - o1)
        val h = Math.sin(dLat / 2).let { it * it } + Math.cos(Math.toRadians(a1)) * Math.cos(Math.toRadians(a2)) * Math.sin(dLon / 2).let { it * it }
        return 2 * r * Math.asin(Math.sqrt(h))
    }
    private fun sha256(s: String) = MessageDigest.getInstance("SHA-256").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }

    // ── on-device track store ───────────────────────────────────────────────────────────
    private val lock = Any()
    @Volatile private var last: Fix? = null
    private fun file(c: Context) = File(c.filesDir, "hone-node/track.jsonl")

    /** Feed every location fix; keeps the thinned track on the device. */
    fun onFix(c: Context, l: Location): Unit = synchronized(lock) {
        val f = Fix(System.currentTimeMillis() - (android.os.SystemClock.elapsedRealtimeNanos() - l.elapsedRealtimeNanos) / 1_000_000,
            l.latitude, l.longitude, l.accuracy, if (l.hasSpeed()) l.speed else 0f)
        if (f.accM > 100f || !keep(last, f)) return
        last = f
        runCatching {
            file(c).apply { parentFile?.mkdirs() }.appendText(RawFmt.json(mapOf("t" to f.tMs, "lat" to f.lat, "lon" to f.lon,
                "acc" to f.accM.toDouble(), "spd" to f.speed.toDouble())) + "\n")
        }
        Unit
    }

    /** Fixes between [fromMs] and [toMs], oldest first. */
    fun track(c: Context, fromMs: Long, toMs: Long): List<Fix> = synchronized(lock) {
        runCatching { file(c).takeIf { it.exists() }?.readLines() }.getOrNull().orEmpty().mapNotNull { l ->
            runCatching {
                val j = org.json.JSONObject(l); val t = j.getLong("t")
                if (t in fromMs..toMs) Fix(t, j.getDouble("lat"), j.getDouble("lon"), j.getDouble("acc").toFloat(), j.optDouble("spd", 0.0).toFloat()) else null
            }.getOrNull()
        }
    }

    /** Keep 30 days. */
    fun prune(c: Context): Unit = synchronized(lock) {
        val f = file(c); if (!f.exists() || f.length() < 512 * 1024) return
        val cut = System.currentTimeMillis() - 30L * 86_400_000
        runCatching { f.writeText(f.readLines().filter { l -> (Regex("\"t\":(\\d+)").find(l)?.groupValues?.get(1)?.toLongOrNull() ?: 0) >= cut }
            .joinToString("") { it + "\n" }) }
        Unit
    }
}

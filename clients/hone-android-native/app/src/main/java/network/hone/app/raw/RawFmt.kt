package network.hone.app.raw

import java.security.MessageDigest
import java.time.Instant
import java.time.format.DateTimeFormatter
import java.time.temporal.ChronoUnit

/**
 * hone.raw v1 (Josh 2026-11-10 10:43): change-based, self-describing records, one JSON object per
 * line. Same format as services/hone-raw/hone_rawfmt.py; the schema registry is
 * services/hone-raw/hone-raw-registry.json (served at honemesh.net/api/raw/schema).
 *
 *  hone.raw.hdr@1  header (device, owner, class, firmware, model, sensors, window, area, registry)
 *  hone.raw.kf@2   keyframe: the full state, at boot, each UTC day and after a gap
 *  hone.raw.ev@2   appeared | disappeared | changed | unchanged | gap
 *  hone.raw.obs@1  payload that is not state (a relayed signed Flipper frame, a pairing)
 * Every record carries provenance: dev, cls, sen, why, loc ([RawFmt.PROV]).
 *
 * A reader rebuilds any moment from the last keyframe plus later events ([rebuild]); the
 * encoder's [ChangeEncoder.reported] is exactly that state. Pure Kotlin (JVM unit-tested).
 */
object RawFmt {
    const val SCHEMA_HDR = "hone.raw.hdr@1"
    const val SCHEMA_KF = "hone.raw.kf@2"
    const val SCHEMA_EV = "hone.raw.ev@2"
    const val SCHEMA_OBS = "hone.raw.obs@1"
    /** Provenance every record carries (Josh 11:22): device, class, sensor, why, location source. */
    val PROV = listOf("dev", "cls", "sen", "why", "loc")

    /** Sensor id for a key: exact key, then its family ("wifi" for wifi:x, "env" for env.rh_pct). */
    fun sensorOf(k: String, sensors: Map<String, String>): String =
        sensors[k] ?: (if (':' in k) k.substringBefore(':') else k.substringBefore('.')).let { sensors[it] ?: it }

    /** The first record lacking provenance, or null. Nothing without it may be sent. */
    fun checkProv(records: List<Map<String, Any?>>): Map<String, Any?>? = records.firstOrNull { r ->
        if (r["s"] == SCHEMA_HDR) listOf("dev", "cls", "win").any { r[it] == null || r[it] == "" } || "area" !in r
        else PROV.any { r[it] == null || r[it] == "" }
    }
    val EVENTS = setOf("appeared", "disappeared", "changed", "unchanged", "gap")
    val GAP_REASONS = setOf("offline", "permission_denied", "speech_deleted", "no_signal", "sensor_absent", "app_stopped")

    /** Deadbands per key (exact) or family prefix ("wifi:"). abs in the key's unit, rel = fraction. */
    data class Band(val abs: Double = 0.0, val rel: Double = 0.0)
    val DEADBANDS: Map<String, Band> = linkedMapOf(
        "wifi:" to Band(abs = 6.0), "ble:" to Band(abs = 6.0), "lora:" to Band(abs = 6.0), "flipper:" to Band(abs = 6.0),
        "env.air_temp_c" to Band(abs = 0.15), "env.rh_pct" to Band(abs = 1.0), "env.pressure_pa" to Band(abs = 10.0),
        "env.light_lux" to Band(rel = 0.10), "dev.temp_c" to Band(abs = 1.0),
    )
    /** Sampled every window even when unchanged, so daily curves are complete. */
    fun forced(k: String) = k.startsWith("env.") || k == "dev.temp_c"

    fun band(k: String): Band = DEADBANDS[k] ?: DEADBANDS.entries.firstOrNull { it.key.endsWith(":") && k.startsWith(it.key) }?.value ?: Band()

    fun moved(k: String, prev: Double, v: Double): Boolean {
        val b = band(k)
        return if (b.rel > 0) Math.abs(v - prev) > b.rel * maxOf(Math.abs(prev), 1e-9) else Math.abs(v - prev) > b.abs
    }

    private val ISO = DateTimeFormatter.ISO_INSTANT
    fun rfc3339(ms: Long): String = ISO.format(Instant.ofEpochMilli(ms).truncatedTo(ChronoUnit.MILLIS)).let {
        if (it.contains('.')) it else it.removeSuffix("Z") + ".000Z"
    }
    fun parse(t: String): Long = Instant.parse(t).toEpochMilli()

    /** Public token for a personal identifier (BSSID, SSID, MAC): never the identifier itself. */
    fun token(vararg parts: String, n: Int = 12): String =
        MessageDigest.getInstance("SHA-256").digest(parts.joinToString("|").toByteArray()).joinToString("") { "%02x".format(it) }.take(n)

    // ── JSON (sorted keys, compact; numbers as written by Python json.dumps) ────────────
    fun json(v: Any?): String = StringBuilder().also { write(it, v) }.toString()
    fun jsonl(records: List<Map<String, Any?>>): ByteArray =
        records.joinToString("") { json(it) + "\n" }.toByteArray(Charsets.UTF_8)

    private fun write(sb: StringBuilder, v: Any?) {
        when (v) {
            null -> sb.append("null")
            is Boolean -> sb.append(v)
            is Int, is Long, is Short, is Byte -> sb.append(v.toString())
            is Double, is Float -> {
                val d = (v as Number).toDouble()
                require(!d.isNaN() && !d.isInfinite()) { "no NaN/inf in raw records" }
                sb.append(if (d == Math.rint(d) && Math.abs(d) < 1e15) d.toLong().toString() else d.toString())
            }
            is String -> str(sb, v)
            is Map<*, *> -> {
                sb.append('{')
                v.entries.map { it.key.toString() to it.value }.sortedBy { it.first }.forEachIndexed { i, (k, x) ->
                    if (i > 0) sb.append(','); str(sb, k); sb.append(':'); write(sb, x)
                }
                sb.append('}')
            }
            is Iterable<*> -> { sb.append('['); v.forEachIndexed { i, x -> if (i > 0) sb.append(','); write(sb, x) }; sb.append(']') }
            else -> str(sb, v.toString())
        }
    }

    private fun str(sb: StringBuilder, s: String) {
        sb.append('"')
        for (c in s) when (c) {
            '"' -> sb.append("\\\""); '\\' -> sb.append("\\\\"); '\n' -> sb.append("\\n"); '\r' -> sb.append("\\r")
            '\t' -> sb.append("\\t"); '\b' -> sb.append("\\b"); '\u000c' -> sb.append("\\f")
            else -> if (c < ' ') sb.append("\\u%04x".format(c.code)) else sb.append(c)
        }
        sb.append('"')
    }

    /** One entry of the state: value plus attributes. */
    data class Entry(val v: Double, val a: Map<String, Any?> = emptyMap())

    /**
     * State at [untilMs] (null = end): last keyframe at or before it, then appeared/changed set,
     * disappeared deletes, unchanged and gap change nothing. Null before the first keyframe.
     */
    @Suppress("UNCHECKED_CAST")
    fun rebuild(records: List<Map<String, Any?>>, untilMs: Long? = null): Map<String, Entry>? {
        var state: LinkedHashMap<String, Entry>? = null
        for (r in records) {
            if (untilMs != null && parse(r["t"] as String) > untilMs) break
            when (r["s"]) {
                SCHEMA_KF -> state = LinkedHashMap<String, Entry>().apply {
                    (r["state"] as Map<String, Map<String, Any?>>).forEach { (k, e) ->
                        put(k, Entry((e["v"] as Number).toDouble(), (e["a"] as? Map<String, Any?>) ?: emptyMap()))
                    }
                }
                SCHEMA_EV -> state?.let { st ->
                    val k = r["k"] as String
                    when (r["e"]) {
                        "appeared", "changed" -> st[k] = Entry((r["v"] as Number).toDouble(),
                            (r["a"] as? Map<String, Any?>) ?: st[k]?.a ?: emptyMap())
                        "disappeared" -> st.remove(k)
                    }
                }
            }
        }
        return state
    }
}

/**
 * Turns observations into keyframes and change events (see [RawFmt]). Not thread-safe: callers lock.
 * [dev], [cls] and the [sensors] table give every record its provenance; [loc] is the current area source.
 */
class ChangeEncoder(private val dev: String, private val cls: String, private val sensors: Map<String, String> = emptyMap(),
                    var loc: String = "none", private val miss: Int = 2) {
    init { require(dev.isNotEmpty() && cls.isNotEmpty()) { "provenance: dev and cls are required" } }
    val reported = LinkedHashMap<String, RawFmt.Entry>()
    private val changedAt = HashMap<String, Long>()
    private val missing = HashMap<String, Int>()
    private val out = ArrayList<Map<String, Any?>>()

    private fun prov(r: MutableMap<String, Any?>, k: String, why: String) {
        r["dev"] = dev; r["cls"] = cls; r["sen"] = RawFmt.sensorOf(k, sensors); r["why"] = why; r["loc"] = loc.ifEmpty { "none" }
    }

    private fun ev(tMs: Long, e: String, k: String, why: String, vararg kv: Pair<String, Any?>) {
        val r = LinkedHashMap<String, Any?>()
        r["s"] = RawFmt.SCHEMA_EV; r["t"] = RawFmt.rfc3339(tMs); r["e"] = e; r["k"] = k
        prov(r, k, why)
        kv.forEach { (a, b) -> if (b != null) r[a] = b }
        out.add(r)
    }

    /** A payload record (not state), e.g. a Flipper's signed frame relayed unaltered. */
    fun obs(tMs: Long, k: String, why: String, fields: Map<String, Any?>) {
        val r = LinkedHashMap<String, Any?>()
        r["s"] = RawFmt.SCHEMA_OBS; r["t"] = RawFmt.rfc3339(tMs); r["k"] = k
        prov(r, k, why)
        fields.forEach { (a, b) -> if (b != null) r[a] = b }
        out.add(r)
    }

    fun keyframe(tMs: Long, why: String) {
        out.add(mapOf("s" to RawFmt.SCHEMA_KF, "t" to RawFmt.rfc3339(tMs), "dev" to dev, "cls" to cls, "sen" to "*",
            "why" to "keyframe:$why", "loc" to loc.ifEmpty { "none" }, "state" to reported.toSortedMap().mapValues { (_, e) -> mapOf("v" to e.v, "a" to e.a) }))
        reported.keys.forEach { changedAt[it] = tMs }
    }

    /** A full scan of one family; [seen] keys start with "$family:". */
    fun observeSet(tMs: Long, family: String, seen: Map<String, Pair<Double, Map<String, Any?>>>) {
        for ((k, va) in seen) {
            val (v, a) = va
            missing.remove(k)
            val cur = reported[k]
            if (cur == null) {
                reported[k] = RawFmt.Entry(v, a); changedAt[k] = tMs
                ev(tMs, "appeared", k, "scan", "v" to v, "a" to a.ifEmpty { null })
            } else if (RawFmt.moved(k, cur.v, v)) {
                ev(tMs, "changed", k, "scan:deadband", "v" to v, "prev" to cur.v)
                reported[k] = cur.copy(v = v); changedAt[k] = tMs
            }
        }
        val gone = reported.keys.filter { it.startsWith("$family:") && it !in seen }
        for (k in gone) {
            val n = (missing[k] ?: 0) + 1
            missing[k] = n
            if (n >= miss) {
                val prev = reported.remove(k)!!.v
                missing.remove(k); changedAt[k] = tMs
                ev(tMs, "disappeared", k, "scan:missing_$miss", "prev" to prev)
            }
        }
    }

    /** One measured quantity (climate, device temperature). */
    fun observe(tMs: Long, k: String, v: Double, a: Map<String, Any?> = emptyMap()) {
        val cur = reported[k]
        if (cur == null) {
            reported[k] = RawFmt.Entry(v, a); changedAt[k] = tMs
            ev(tMs, "appeared", k, "sample", "v" to v, "a" to a.ifEmpty { null })
        } else if (RawFmt.moved(k, cur.v, v)) {
            ev(tMs, "changed", k, "sample:deadband", "v" to v, "prev" to cur.v)
            reported[k] = cur.copy(v = v); changedAt[k] = tMs
        }
    }

    fun gap(fromMs: Long, toMs: Long, family: String, reason: String) {
        require(reason in RawFmt.GAP_REASONS) { "unknown gap reason $reason" }
        ev(toMs, "gap", "$family:*", "gap:$reason", "reason" to reason, "from" to RawFmt.rfc3339(fromMs), "to" to RawFmt.rfc3339(toMs))
    }

    /** Explicit markers at the end of a window, and the forced climate samples. */
    fun endWindow(tMs: Long, families: List<String>) {
        for (f in families) {
            val ks = reported.keys.filter { it.startsWith("$f:") }
            if (ks.isNotEmpty()) ev(tMs, "unchanged", "$f:*", "marker:window_end", "since" to RawFmt.rfc3339(ks.minOf { changedAt[it] ?: tMs }), "n" to ks.size)
        }
        for ((k, e) in reported.toSortedMap()) if (RawFmt.forced(k))
            ev(tMs, "unchanged", k, "forced_sample:15min", "v" to e.v, "since" to RawFmt.rfc3339(changedAt[k] ?: tMs), "forced" to true)
    }

    fun take(): List<Map<String, Any?>> = ArrayList(out).also { out.clear() }
}

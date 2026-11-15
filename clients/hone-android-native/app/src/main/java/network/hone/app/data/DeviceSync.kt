package network.hone.app.data

import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * Sync before submit (chain spec-universal-reputation §6b, hone-node d0cb2373).
 *
 * The chain keeps every device key's committed head {seq, head_hash} (GET /api/device/:key/head,
 * key = the commit's sensor_id). Once its gate (REPUTATION_EPOCH) is active, a SensorDataCommit /
 * SensorSummary must carry seq > head.seq and prev_head == head.head_hash, both inside the signed
 * message; anything else is rejected with "...; resync". Two different signed commits at one seq
 * are equivocation and slash the device's stake.
 *
 * So every key this phone signs for keeps a local log here: the highest seq it ever signed (spent
 * before signing, so a seq is never used twice) and the chain hashes of its accepted commits.
 * Before the first submit after start, after a network failure, or after a gap, it fetches the
 * chain's head and reconciles it with that log. A chain head that is not in the log means the key
 * also runs somewhere else (a copied phone, an old backup restored): submitting stops for that key
 * and the owner is told; only the owner's re-anchor starts it again, with a new seq after the
 * chain's.
 *
 * While the node has no head endpoint (404, today's live nodes) or its gate is off, commits go out
 * exactly as before: no seq, no prev_head, byte-identical signing messages.
 */
object DeviceLog {
    /** Seqs and accepted heads remembered per key. */
    const val KEEP = 256

    data class Head(val seq: Long, val hash: String)

    data class State(
        val floor: Long = 0,                       // highest seq ever signed (or re-anchored to)
        val accepted: List<Head> = emptyList(),    // own commits the chain accepted, oldest first
        val signed: List<Long> = emptyList(),      // own signed seqs, oldest first
        val wrapped: Boolean = false,              // older seqs were forgotten
        val head: Head? = null,                    // chain head the next commit builds on
        val paused: Boolean = false,
        val reason: String = "",
    )

    enum class Verdict { OK, MISMATCH }

    /** Is the chain's head [h] in this key's own history? */
    fun check(st: State, h: Head?): Verdict {
        if (h == null) return Verdict.OK                                   // nothing committed yet
        if (h.seq > st.floor) return Verdict.MISMATCH                      // a seq we never reached
        st.accepted.firstOrNull { it.seq == h.seq }?.let {
            return if (it.hash.equals(h.hash, ignoreCase = true)) Verdict.OK else Verdict.MISMATCH
        }
        if (h.seq in st.signed) return Verdict.OK                          // ours; answer was lost
        val oldest = (st.signed + st.accepted.map { it.seq }).minOrNull()
        if (st.wrapped && (oldest == null || h.seq < oldest)) return Verdict.OK // too old to tell
        return Verdict.MISMATCH
    }

    fun reconcile(st: State, h: Head?): Pair<State, Verdict> = when (check(st, h)) {
        Verdict.OK -> st.copy(head = h, accepted = learn(st.accepted, h)) to Verdict.OK
        Verdict.MISMATCH -> st.copy(paused = true,
            reason = "chain head ${h?.seq}:${h?.hash?.take(12)} is not in this device's history (local seq ${st.floor})") to
            Verdict.MISMATCH
    }

    /** Spends the next seq. The caller saves the returned state before signing with it. */
    fun reserve(st: State): Pair<State, Long> {
        val seq = st.floor + 1
        val signed = st.signed + seq
        return st.copy(floor = seq, signed = signed.takeLast(KEEP), wrapped = st.wrapped || signed.size > KEEP) to seq
    }

    fun accepted(st: State, seq: Long, hash: String): State {
        val h = Head(seq, hash.lowercase())
        val acc = (st.accepted.filter { it.seq != seq } + h)
        return st.copy(head = h, accepted = acc.takeLast(KEEP), wrapped = st.wrapped || acc.size > KEEP)
    }

    /** Owner's re-anchor: build on the chain's head, next seq above it and above anything signed. */
    fun reanchor(st: State, h: Head?): State = st.copy(
        floor = maxOf(st.floor, h?.seq ?: 0), accepted = listOfNotNull(h), signed = emptyList(),
        wrapped = false, head = h, paused = false, reason = "")

    private fun learn(acc: List<Head>, h: Head?): List<Head> =
        if (h == null || acc.any { it.seq == h.seq }) acc else (acc + h).sortedBy { it.seq }.takeLast(KEEP)

    fun toJson(st: State): String = JSONObject().apply {
        put("v", 1); put("floor", st.floor); put("wrapped", st.wrapped); put("paused", st.paused); put("reason", st.reason)
        put("accepted", JSONArray().apply { st.accepted.forEach { put(JSONArray().put(it.seq).put(it.hash)) } })
        put("signed", JSONArray().apply { st.signed.forEach { put(it) } })
        st.head?.let { put("head", JSONArray().put(it.seq).put(it.hash)) }
    }.toString()

    fun fromJson(s: String): State {
        val j = JSONObject(s)
        val acc = j.optJSONArray("accepted")
        val sig = j.optJSONArray("signed")
        val hd = j.optJSONArray("head")
        return State(
            floor = j.optLong("floor"),
            accepted = (0 until (acc?.length() ?: 0)).map { acc!!.getJSONArray(it).let { a -> Head(a.getLong(0), a.getString(1)) } },
            signed = (0 until (sig?.length() ?: 0)).map { sig!!.getLong(it) },
            wrapped = j.optBoolean("wrapped"),
            head = hd?.let { Head(it.getLong(0), it.getString(1)) },
            paused = j.optBoolean("paused"),
            reason = j.optString("reason"),
        )
    }
}

/** What the chain says about a key's head. */
sealed class HeadFetch {
    /** No node answered. */
    data object Unreachable : HeadFetch()
    /** The node has no head endpoint (404) or its gate is off: commits without seq, as before. */
    data object Legacy : HeadFetch()
    /** Gate active: the committed head, or null when the key has none yet. */
    data class Active(val head: DeviceLog.Head?) : HeadFetch()
}

fun interface HeadSource { fun fetch(key: String): HeadFetch }

interface DeviceLogStore {
    fun load(key: String): DeviceLog.State
    fun save(key: String, st: DeviceLog.State)
    fun all(): Map<String, DeviceLog.State>
}

/** One JSON file per key under [dir] (hone-node/devlog), written atomically. */
class FileDeviceLogStore(private val dir: File) : DeviceLogStore {
    private fun file(key: String) = File(dir, key.replace(Regex("[^A-Za-z0-9._@-]"), "_").take(150) + ".json")
    override fun load(key: String): DeviceLog.State =
        runCatching { DeviceLog.fromJson(file(key).readText()) }.getOrDefault(DeviceLog.State())
    override fun save(key: String, st: DeviceLog.State) {
        dir.mkdirs()
        val f = file(key)
        val tmp = File(dir, f.name + ".tmp")
        tmp.writeText(JSONObject(DeviceLog.toJson(st)).put("key", key).toString())
        if (!tmp.renameTo(f)) { f.writeText(tmp.readText()); tmp.delete() }
    }
    override fun all(): Map<String, DeviceLog.State> = (dir.listFiles { f -> f.name.endsWith(".json") } ?: emptyArray())
        .mapNotNull { f -> runCatching { val t = f.readText(); JSONObject(t).getString("key") to DeviceLog.fromJson(t) }.getOrNull() }
        .toMap()
}

/** GET /api/device/<key>/head through [get] (code, body), null when no node answered. */
class HttpHeadSource(private val get: (String) -> Pair<Int, String?>?) : HeadSource {
    override fun fetch(key: String): HeadFetch {
        val (code, body) = get("/api/device/${pathEncode(key)}/head") ?: return HeadFetch.Unreachable
        if (code in 400..499) return HeadFetch.Legacy                      // 404: node without the endpoint
        if (code !in 200..299 || body == null) return HeadFetch.Unreachable
        return parse(body)
    }

    companion object {
        fun parse(body: String): HeadFetch {
            val j = runCatching { JSONObject(body) }.getOrNull() ?: return HeadFetch.Unreachable
            if (!j.optBoolean("active", false)) return HeadFetch.Legacy
            val h = j.optJSONObject("head") ?: return HeadFetch.Active(null)
            val hash = h.optString("head_hash")
            if (!h.has("seq") || hash.isEmpty()) return HeadFetch.Active(null)
            return HeadFetch.Active(DeviceLog.Head(h.getLong("seq"), hash.lowercase()))
        }

        fun pathEncode(s: String): String = buildString {
            for (b in s.toByteArray(Charsets.UTF_8)) {
                val c = b.toInt() and 0xff
                if ((c in 'a'.code..'z'.code) || (c in 'A'.code..'Z'.code) || (c in '0'.code..'9'.code) || c.toChar() in "-._~@")
                    append(c.toChar()) else append("%%%02X".format(c))
            }
        }
    }
}

/**
 * Submits through the device log. [submit]'s `send(seq, prevHead)` signs and posts one commit:
 * both null = legacy (no seq), seq set = the chain's sync rule (prevHead null on a key's first commit).
 */
class DeviceSyncEngine(
    private val store: DeviceLogStore,
    private val source: HeadSource,
    private val onMismatch: (key: String, reason: String) -> Unit,
    private val clock: () -> Long = System::currentTimeMillis,
) {
    data class Post(val code: Int, val body: String)   // code 0 = no node answered

    sealed class Outcome {
        data class Accepted(val hash: String, val seq: Long?) : Outcome()
        data class Rejected(val code: Int, val body: String) : Outcome()
        data object Offline : Outcome()
        /** Submissions for this key are paused (history mismatch); keep the data, send nothing. */
        data object Paused : Outcome()
    }

    private val synced = HashMap<String, Long>()       // key -> last time the head was known good
    private val legacyUntil = HashMap<String, Long>()  // key -> recheck the endpoint after

    fun submit(key: String, send: (seq: Long?, prevHead: String?) -> Post): Outcome = synchronized(lock(key)) {
        var st = store.load(key)
        if (st.paused) return Outcome.Paused
        val now = clock()
        if ((legacyUntil[key] ?: 0L) > now) return legacy(send)
        if (now - (synced[key] ?: Long.MIN_VALUE / 2) >= GAP_MS) {
            when (val r = sync(key, st)) {
                is SyncResult.Done -> st = r.st
                SyncResult.Legacy -> return legacy(send)
                SyncResult.Offline -> return Outcome.Offline
                SyncResult.Paused -> return Outcome.Paused
            }
        }
        for (attempt in 0..1) {
            val (spent, seq) = DeviceLog.reserve(st)
            store.save(key, spent)                                   // the seq is spent before it is signed
            st = spent
            val r = send(seq, st.head?.hash)
            if (r.code == 0) { synced.remove(key); return Outcome.Offline }
            val j = runCatching { JSONObject(r.body) }.getOrNull()
            if (r.code in 200..299 && j?.optBoolean("accepted") == true) {
                val hash = j.optString("hash")
                if (hash.isNotEmpty() && hash != "null") { st = DeviceLog.accepted(st, seq, hash); store.save(key, st) }
                synced[key] = clock()
                return Outcome.Accepted(hash, seq)
            }
            val err = j?.optString("error")?.takeIf { it.isNotEmpty() && it != "null" } ?: r.body
            if (err.contains("equivocation", ignoreCase = true)) {
                st = st.copy(paused = true, reason = "the chain reports equivocation at seq $seq: another device signs with this key")
                store.save(key, st); onMismatch(key, st.reason)
                return Outcome.Paused
            }
            if (attempt == 0 && isResync(err)) {
                synced.remove(key)
                when (val s = sync(key, st)) {
                    is SyncResult.Done -> { st = s.st; continue }           // retry once, new seq
                    SyncResult.Legacy -> return legacy(send)
                    SyncResult.Offline -> return Outcome.Offline
                    SyncResult.Paused -> return Outcome.Paused
                }
            }
            return Outcome.Rejected(r.code, r.body)
        }
        return Outcome.Rejected(0, "unreachable")
    }

    /** Owner action after a mismatch: build on the chain's current head with a new seq above it. */
    fun reanchor(key: String): Boolean = synchronized(lock(key)) {
        val st = store.load(key)
        when (val f = source.fetch(key)) {
            HeadFetch.Unreachable -> false
            HeadFetch.Legacy -> { store.save(key, DeviceLog.reanchor(st, null)); true }
            is HeadFetch.Active -> { store.save(key, DeviceLog.reanchor(st, f.head)); synced[key] = clock(); true }
        }
    }

    /** Forget that the head was known (reconnect): the next submit fetches it first. */
    fun invalidate(key: String) { synchronized(lock(key)) { synced.remove(key) } }

    private sealed class SyncResult {
        data class Done(val st: DeviceLog.State) : SyncResult()
        data object Legacy : SyncResult()
        data object Offline : SyncResult()
        data object Paused : SyncResult()
    }

    private fun sync(key: String, st: DeviceLog.State): SyncResult = when (val f = source.fetch(key)) {
        HeadFetch.Unreachable -> SyncResult.Offline
        HeadFetch.Legacy -> { legacyUntil[key] = clock() + LEGACY_RECHECK_MS; SyncResult.Legacy }
        is HeadFetch.Active -> {
            legacyUntil.remove(key)
            val (s2, v) = DeviceLog.reconcile(st, f.head)
            store.save(key, s2)
            if (v == DeviceLog.Verdict.MISMATCH) { onMismatch(key, s2.reason); SyncResult.Paused }
            else { synced[key] = clock(); SyncResult.Done(s2) }
        }
    }

    private fun legacy(send: (Long?, String?) -> Post): Outcome {
        val r = send(null, null)
        if (r.code == 0) return Outcome.Offline
        val j = runCatching { JSONObject(r.body) }.getOrNull()
        return if (r.code in 200..299 && j?.optBoolean("accepted") == true) Outcome.Accepted(j.optString("hash"), null)
        else Outcome.Rejected(r.code, r.body)
    }

    companion object {
        /** Refetch the head before submitting after this long without a good submit. */
        const val GAP_MS = 10 * 60_000L
        /** While the node has no endpoint / gate off, ask again this often. */
        const val LEGACY_RECHECK_MS = 30 * 60_000L

        private val locks = HashMap<String, Any>()
        private fun lock(key: String): Any = synchronized(locks) { locks.getOrPut(key) { Any() } }

        fun isResync(err: String): Boolean = listOf("resync", "already committed", "stale commit", "does not extend")
            .any { err.contains(it, ignoreCase = true) }

        /** The signing message with the device-log fields: appended, in this order, only when present
         *  (hone-node tx.rs with_device_log, serde_json preserve_order). */
        fun withDeviceLog(message: String, seq: Long?, prevHead: String?): String {
            if (seq == null) return message
            require(message.endsWith("}"))
            val prev = prevHead?.let { ",\"prev_head\":\"$it\"" }.orEmpty()
            return message.dropLast(1) + ",\"seq\":$seq$prev}"
        }
    }
}

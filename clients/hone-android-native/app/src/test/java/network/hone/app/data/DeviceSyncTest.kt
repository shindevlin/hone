package network.hone.app.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DeviceSyncTest {
    private val H = DeviceLog.Head(5, "aa".repeat(32))

    private class MemStore : DeviceLogStore {
        val m = HashMap<String, DeviceLog.State>()
        override fun load(key: String) = m[key] ?: DeviceLog.State()
        override fun save(key: String, st: DeviceLog.State) { m[key] = st }
        override fun all() = m.toMap()
    }

    /** A chain stand-in: enforces the head rule like hone-node rep::device_check. */
    private class FakeChain(var mode: String = "active") {
        var head: DeviceLog.Head? = null
        val seen = HashMap<Long, String>()
        var n = 0
        val source = HeadSource { if (mode == "down") HeadFetch.Unreachable else if (mode == "404") HeadFetch.Legacy else HeadFetch.Active(head) }
        fun post(seq: Long?, prev: String?, payload: String): DeviceSyncEngine.Post {
            if (mode == "down") return DeviceSyncEngine.Post(0, "")
            if (mode == "404") return DeviceSyncEngine.Post(200, """{"accepted":true,"hash":"legacy${n++}"}""")
            if (seq == null) return err("commit has no seq/prev_head; fetch GET /api/device/k/head and resync")
            seen[seq]?.let { return err(if (it != payload) "equivocation, a different commit is already on chain at seq $seq" else "seq $seq is already committed") }
            val h = head
            if (h == null && prev != null) return err("no committed head yet, prev_head must be empty; resync")
            if (h != null && seq <= h.seq) return err("stale commit seq $seq, head is seq ${h.seq}; resync")
            if (h != null && prev != h.hash) return err("commit does not extend the committed head; resync")
            val hash = "%064x".format(1000L + n++)
            seen[seq] = payload
            head = DeviceLog.Head(seq, hash)
            return DeviceSyncEngine.Post(200, """{"hash":"$hash","accepted":true,"status":"pending","error":null}""")
        }
        private fun err(e: String) = DeviceSyncEngine.Post(400, """{"hash":null,"accepted":false,"error":"device 'k': $e"}""")
    }

    private fun engine(chain: FakeChain, store: DeviceLogStore, alerts: MutableList<String>, now: () -> Long = { 0L }) =
        DeviceSyncEngine(store, chain.source, { k, _ -> alerts.add(k) }, now)

    @Test fun checkRules() {
        val st = DeviceLog.State(floor = 7, accepted = listOf(H), signed = listOf(5, 6, 7))
        assertEquals(DeviceLog.Verdict.OK, DeviceLog.check(st, null))
        assertEquals(DeviceLog.Verdict.OK, DeviceLog.check(st, H))
        assertEquals(DeviceLog.Verdict.OK, DeviceLog.check(st, DeviceLog.Head(6, "bb")))       // ours, answer lost
        assertEquals(DeviceLog.Verdict.MISMATCH, DeviceLog.check(st, DeviceLog.Head(8, "bb")))  // chain ahead of us
        assertEquals(DeviceLog.Verdict.MISMATCH, DeviceLog.check(st, DeviceLog.Head(5, "bb")))  // our seq, other hash
        assertEquals(DeviceLog.Verdict.MISMATCH, DeviceLog.check(st, DeviceLog.Head(3, "bb")))  // never signed 3
        assertEquals(DeviceLog.Verdict.OK, DeviceLog.check(st.copy(wrapped = true), DeviceLog.Head(3, "bb"))) // too old to tell
        assertEquals(DeviceLog.Verdict.MISMATCH, DeviceLog.check(DeviceLog.State(), H))         // fresh log, chain has a head
    }

    @Test fun reserveNeverReusesAndJsonRoundTrips() {
        var st = DeviceLog.State()
        val seqs = (1..300).map { val (s, q) = DeviceLog.reserve(st); st = s; q }
        assertEquals(seqs.distinct().size, 300)
        assertTrue(st.wrapped && st.signed.size == DeviceLog.KEEP)
        st = DeviceLog.accepted(st, 300, "AB".repeat(32))
        val back = DeviceLog.fromJson(DeviceLog.toJson(st))
        assertEquals(st, back)
        val re = DeviceLog.reanchor(st, DeviceLog.Head(250, "cc"))
        assertEquals(300, re.floor)                                    // never goes back
        assertEquals(301L, DeviceLog.reserve(re).second)
        assertEquals(DeviceLog.Verdict.OK, DeviceLog.check(re, DeviceLog.Head(250, "cc")))
    }

    @Test fun legacyWhileTheNodeHasNoEndpoint() {
        val chain = FakeChain("404"); val store = MemStore(); val alerts = ArrayList<String>()
        val e = engine(chain, store, alerts)
        var got: Pair<Long?, String?>? = null
        val out = e.submit("k") { seq, prev -> got = seq to prev; chain.post(seq, prev, "x") }
        assertTrue(out is DeviceSyncEngine.Outcome.Accepted)
        assertEquals(null to null, got)
        assertEquals(0L, store.load("k").floor)                         // no seq spent
    }

    @Test fun chainsCommitsOnTheHead() {
        val chain = FakeChain(); val store = MemStore(); val alerts = ArrayList<String>()
        val e = engine(chain, store, alerts)
        repeat(3) { i -> assertTrue(e.submit("k") { s, p -> chain.post(s, p, "p$i") } is DeviceSyncEngine.Outcome.Accepted) }
        assertEquals(3L, chain.head!!.seq)
        assertEquals(chain.head, store.load("k").head)
        assertTrue(alerts.isEmpty())
    }

    @Test fun resyncRetriesOnceWithANewSeq() {
        val chain = FakeChain(); val store = MemStore(); val alerts = ArrayList<String>()
        val e = engine(chain, store, alerts)
        e.submit("k") { s, p -> chain.post(s, p, "a") }
        // our accepted commit did not make it into a block: the chain head is still none
        chain.head = null; chain.seen.clear()
        val used = ArrayList<Long?>()
        val out = e.submit("k") { s, p -> used.add(s); chain.post(s, p, "b") }
        assertTrue(out is DeviceSyncEngine.Outcome.Accepted)
        assertEquals(listOf(2L, 3L), used)                              // 2 refused (stale prev), 3 new
        assertNull(store.load("k").let { if (it.paused) it else null })
    }

    @Test fun headNotInHistoryPausesAndAlerts() {
        val chain = FakeChain(); val alerts = ArrayList<String>()
        val a = MemStore()
        val e1 = engine(chain, a, alerts)
        repeat(2) { e1.submit("k") { s, p -> chain.post(s, p, "one$it") } }
        // a copy made now (backup) runs on, then the original is restored from that backup
        val b = MemStore().apply { m["k"] = a.load("k") }
        repeat(2) { e1.submit("k") { s, p -> chain.post(s, p, "two$it") } }
        val restored = engine(chain, b, alerts)
        var sent = false
        val out = restored.submit("k") { s, p -> sent = true; chain.post(s, p, "three") }
        assertEquals(DeviceSyncEngine.Outcome.Paused, out)
        assertFalse(sent)                                               // nothing signed
        assertEquals(listOf("k"), alerts)
        assertTrue(b.load("k").paused)
        assertEquals(DeviceSyncEngine.Outcome.Paused, restored.submit("k") { s, p -> chain.post(s, p, "x") })
        // owner re-anchors: continues after the chain's head, new seq
        assertTrue(restored.reanchor("k"))
        val used = ArrayList<Long?>()
        assertTrue(restored.submit("k") { s, p -> used.add(s); chain.post(s, p, "four") } is DeviceSyncEngine.Outcome.Accepted)
        assertEquals(listOf(5L), used)
    }

    @Test fun equivocationPauses() {
        val chain = FakeChain(); val alerts = ArrayList<String>()
        val store = MemStore()
        val e = engine(chain, store, alerts)
        e.submit("k") { s, p -> chain.post(s, p, "a") }
        // another instance with this key already got seq 2 on chain with other content
        chain.seen[2] = "other"
        val out = e.submit("k") { s, p -> chain.post(s, p, "b") }
        assertEquals(DeviceSyncEngine.Outcome.Paused, out)
        assertTrue(store.load("k").paused && alerts == listOf("k"))
    }

    @Test fun offlineFetchesAgainAndNeverReusesTheSeq() {
        val chain = FakeChain(); val store = MemStore(); val alerts = ArrayList<String>()
        val e = engine(chain, store, alerts)
        e.submit("k") { s, p -> chain.post(s, p, "a") }
        val used = ArrayList<Long?>()
        assertEquals(DeviceSyncEngine.Outcome.Offline, e.submit("k") { s, _ -> used.add(s); DeviceSyncEngine.Post(0, "") })
        chain.mode = "down"
        assertEquals(DeviceSyncEngine.Outcome.Offline, e.submit("k") { s, _ -> used.add(s); DeviceSyncEngine.Post(0, "") })
        chain.mode = "active"
        assertTrue(e.submit("k") { s, p -> used.add(s); chain.post(s, p, "b") } is DeviceSyncEngine.Outcome.Accepted)
        assertEquals(listOf(2L, 3L), used)                              // 2 spent while offline, never reused
    }

    @Test fun wireFieldsAppendedOnlyWhenPresent() {
        val m = "{\"a\":1}"
        assertEquals(m, DeviceSyncEngine.withDeviceLog(m, null, null))
        assertEquals("{\"a\":1,\"seq\":7}", DeviceSyncEngine.withDeviceLog(m, 7, null))
        assertEquals("{\"a\":1,\"seq\":7,\"prev_head\":\"ff\"}", DeviceSyncEngine.withDeviceLog(m, 7, "ff"))
        assertTrue(DeviceSyncEngine.isResync("device 'x': stale commit seq 3, head is seq 4; resync"))
        assertFalse(DeviceSyncEngine.isResync("bad signature"))
    }

    @Test fun headEndpointParsing() {
        assertEquals(HeadFetch.Legacy, HttpHeadSource.parse("""{"key":"k","head":null,"active":false}"""))
        assertEquals(HeadFetch.Active(null), HttpHeadSource.parse("""{"key":"k","head":null,"active":true}"""))
        assertEquals(HeadFetch.Active(DeviceLog.Head(9, "ab")),
            HttpHeadSource.parse("""{"key":"k","head":{"seq":9,"head_hash":"AB","epoch":3},"active":true}"""))
        assertEquals(HeadFetch.Legacy, HttpHeadSource { 404 to null }.fetch("k"))
        assertEquals(HeadFetch.Unreachable, HttpHeadSource { null }.fetch("k"))
        assertEquals("josh%2Fx%20y@ab", HttpHeadSource.pathEncode("josh/x y@ab"))
    }
}

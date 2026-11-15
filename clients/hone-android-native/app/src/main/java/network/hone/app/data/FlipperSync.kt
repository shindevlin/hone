package network.hone.app.data

import android.content.Context
import android.hardware.usb.UsbManager
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeout
import network.hone.app.fw.BleFlipper
import network.hone.app.fw.FlipperTransport
import network.hone.app.fw.RpcSession
import network.hone.app.fw.UsbFlipper
import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.zip.CRC32

/**
 * Sync before submit for HONE Flippers (firmware >= 2.3.0, applications/services/hone/hone_sync.h).
 *
 * A Flipper signs its own commits (sensor ids flipper-<devid>-<group>); phones only carry them. The
 * chain's head hash includes the epoch the node stamps on, so the Flipper cannot compute it: the
 * owner's phone fetches the heads of the Flipper's keys and writes them to the Flipper over the
 * Flipper RPC (USB if plugged in and allowed, else the bonded Bluetooth link):
 *
 *   /ext/apps_data/hone/chain_head.in =
 *     "HSH1" | flags u8 | n u8 | 0 u16 | devid[8] | n x (group u8 | has u8 | seq u32 LE | hash[32]) | crc32 u32 LE
 *     flags: 1 = the chain enforces device heads (sign seq'd commits), 2 = owner's re-anchor.
 *
 * With sync on, the Flipper broadcasts 'HS' commits (144 B): devid8 | group | count u16 | batch_hash32 |
 * seq u32 | has_prev | prev_head32 | sig64, signed over the chain message with seq / prev_head, and
 * signs the next one for a group only after a delivered head shows the last one landed. When a
 * delivered head is not in its history it stops committing, shows "HONE: history mismatch - open app"
 * and sets announce flag bit 2; re-anchor (flag 2) starts it again above the chain's head.
 */
object FlipperSync {
    private const val TAG = "HoneFlipperSync"
    /** Commit group index = byte in the frame = sensor id suffix (firmware hone_group_name). */
    val GROUPS = listOf("subghz", "wifi", "tags", "ir", "blecrowd", "sightings")
    val TYPES = listOf("continuous", "continuous", "event", "event", "continuous", "event")
    const val S_LEN = 144
    const val LEGACY_LEN = 107
    const val F_ON = 1
    const val F_REANCHOR = 2
    const val ANN_MISMATCH = 1 shl 2
    const val ANN_SYNC_ON = 1 shl 3
    const val IN_PATH = "/ext/apps_data/hone/chain_head.in"
    private const val PERIOD_MS = 10 * 60_000L
    private const val AFTER_RELAY_MS = 20_000L

    data class SCommit(val devId: String, val group: Int, val count: Int, val hash: String,
                       val seq: Long, val prevHead: String?, val sig: ByteArray) {
        val sensorId get() = "flipper-$devId-${GROUPS[group]}"
    }

    data class Entry(val group: Int, val head: DeviceLog.Head?)

    /** Parses an 'HS' commit (144 bytes); null if malformed. The signature is checked by the caller. */
    fun parseS(d: ByteArray): SCommit? {
        if (d.size != S_LEN) return null
        val group = d[8].toInt() and 0xFF
        if (group >= GROUPS.size) return null
        val bb = ByteBuffer.wrap(d).order(ByteOrder.LITTLE_ENDIAN)
        val count = bb.getShort(9).toInt() and 0xFFFF
        val seq = bb.getInt(43).toLong() and 0xFFFFFFFFL
        val hasPrev = d[47].toInt() != 0
        return SCommit(hex(d.copyOfRange(0, 8)), group, count, hex(d.copyOfRange(11, 43)), seq,
            if (hasPrev) hex(d.copyOfRange(48, 80)) else null, d.copyOfRange(80, 144))
    }

    /** The chain's SENSOR_DATA_COMMIT signing message as the Flipper signs it. */
    fun message(sensorId: String, owner: String, hash: String, count: Int, type: String, seq: Long?, prevHead: String?): String =
        DeviceSyncEngine.withDeviceLog(
            "{\"chain_id\":\"hone\",\"type\":\"SENSOR_DATA_COMMIT\",\"sensor_id\":\"$sensorId\",\"owner\":\"$owner\"," +
                "\"batch_hash\":\"$hash\",\"reading_count\":$count,\"sensor_type\":\"$type\",\"signed_by\":\"$owner\"}",
            seq, prevHead)

    /** POST /api/sensor/commit body for a relayed 'HS' commit (prev_head omitted on a first commit). */
    fun body(c: SCommit, owner: String, gateway: String): JSONObject = JSONObject()
        .put("sensor_id", c.sensorId).put("owner", owner).put("batch_hash", c.hash)
        .put("reading_count", c.count).put("sensor_type", TYPES[c.group])
        .put("seq", c.seq).apply { c.prevHead?.let { put("prev_head", it) } }
        .put("signature", hex(c.sig)).apply { if (gateway.isNotEmpty()) put("gateway_account", gateway) }

    /** chain_head.in bytes. */
    fun headFile(flags: Int, devId: ByteArray, entries: List<Entry>): ByteArray {
        require(devId.size == 8 && entries.size <= GROUPS.size)
        val len = 16 + entries.size * 38
        val b = ByteBuffer.allocate(len + 4).order(ByteOrder.LITTLE_ENDIAN)
        b.put("HSH1".toByteArray()).put(flags.toByte()).put(entries.size.toByte()).putShort(0).put(devId)
        for (e in entries) {
            b.put(e.group.toByte()).put(if (e.head != null) 1 else 0).putInt((e.head?.seq ?: 0L).toInt())
            b.put(e.head?.hash?.let { unhex(it) }?.takeIf { it.size == 32 } ?: ByteArray(32))
        }
        val crc = CRC32().apply { update(b.array(), 0, len) }.value
        b.putInt(crc.toInt())
        return b.array()
    }

    /** The bonded (main) Bluetooth address of a Flipper heard on its HONE beacon address: the beacon
     *  address is the main one with bytes 0 and 3 inverted (little-endian; octets 5 and 2 as shown). */
    fun mainAddress(beacon: String): String? {
        val o = beacon.split(":")
        if (o.size != 6) return null
        return o.mapIndexed { i, s -> if (i == 2 || i == 5) "%02X".format(s.toInt(16) xor 0xFF) else s.uppercase() }.joinToString(":")
    }

    // ── Android side ──────────────────────────────────────────────────────────────────────────

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val lastPush = HashMap<String, Long>()
    private val busy = HashSet<String>()
    private val syncOnSeen = HashMap<String, Boolean>()
    private val beaconAddr = HashMap<String, String>()
    private val enrolled = HashMap<String, Pair<Long, Boolean>>() // sensor id -> (at, registered)
    @Volatile var lastResult: String = ""
        private set

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_flipper_sync", Context.MODE_PRIVATE)

    /** Own Flippers that reported a history mismatch (devid -> time). */
    fun mismatched(ctx: Context): Set<String> = prefs(ctx).getStringSet("mismatch", emptySet()) ?: emptySet()

    private fun mine(ctx: Context, owner: String): Boolean =
        owner.isNotEmpty() && owner == network.hone.app.telemetry.Telemetry.account(ctx)

    /** Every verified announce. Only the owner's phone writes heads to its Flipper. */
    fun onAnnounce(ctx: Context, d: HoneDevice, address: String) {
        if (!d.signatureOk || !mine(ctx, d.owner)) return
        synchronized(this) { syncOnSeen[d.deviceId] = d.syncOn; beaconAddr[d.deviceId] = address }
        val p = prefs(ctx)
        val set = mismatched(ctx)
        if (d.historyMismatch && d.deviceId !in set) {
            p.edit().putStringSet("mismatch", set + d.deviceId).apply()
            DeviceSync.notifyMismatch(ctx, "flipper-${d.deviceId}")
        } else if (!d.historyMismatch && d.deviceId in set) {
            p.edit().putStringSet("mismatch", set - d.deviceId).apply()
            DeviceSync.cancel(ctx, "flipper-${d.deviceId}")
        }
        if (!d.historyMismatch) pushSoon(ctx, d.deviceId, PERIOD_MS)
    }

    /** A relayed 'HS' commit was accepted: the Flipper waits for this head before its next commit. */
    fun onRelayAccepted(ctx: Context, devId: String, owner: String) {
        if (mine(ctx, owner)) pushSoon(ctx, devId, AFTER_RELAY_MS)
    }

    /** A relayed 'HS' commit was refused: a stale head (resync) or equivocation. */
    fun onRelayRefused(ctx: Context, devId: String, owner: String, error: String) {
        if (!mine(ctx, owner)) return
        if (error.contains("equivocation", ignoreCase = true)) {
            Log.w(TAG, "flipper $devId: equivocation reported by the chain")
            DeviceSync.notifyMismatch(ctx, "flipper-$devId")
        }
        if (DeviceSyncEngine.isResync(error)) pushSoon(ctx, devId, AFTER_RELAY_MS)
    }

    /** Owner's re-anchor of a Flipper (needs the USB or Bluetooth link). */
    fun reanchor(ctx: Context, devId: String, done: (String) -> Unit) {
        scope.launch {
            val r = push(ctx.applicationContext, devId, reanchor = true)
            if (r.startsWith("sent")) {
                prefs(ctx).edit().putStringSet("mismatch", mismatched(ctx) - devId).apply()
                DeviceSync.cancel(ctx, "flipper-$devId")
            }
            done(r)
        }
    }

    private fun pushSoon(ctx: Context, devId: String, minGapMs: Long) {
        val now = System.currentTimeMillis()
        synchronized(this) {
            if (devId in busy || now - (lastPush[devId] ?: 0L) < minGapMs) return
            busy.add(devId); lastPush[devId] = now
        }
        val app = ctx.applicationContext
        scope.launch {
            try { lastResult = push(app, devId, reanchor = false); Log.i(TAG, "flipper $devId: $lastResult") }
            finally { synchronized(this@FlipperSync) { busy.remove(devId) } }
        }
    }

    private fun registered(sensorId: String): Boolean? {
        enrolled[sensorId]?.let { (t, v) -> if (System.currentTimeMillis() - t < 3_600_000L) return v }
        val (code, _) = ChainHttp.getWithCode("/api/sensor/$sensorId") ?: return null
        val v = code in 200..299
        enrolled[sensorId] = System.currentTimeMillis() to v
        return v
    }

    /** Fetches the chain heads of the Flipper's enrolled keys and writes chain_head.in. */
    private suspend fun push(ctx: Context, devId: String, reanchor: Boolean): String {
        val src = HttpHeadSource { ChainHttp.getWithCode(it) }
        val entries = ArrayList<Entry>()
        var on = true
        var any = false
        for ((g, name) in GROUPS.withIndex()) {
            val key = "flipper-$devId-$name"
            val reg = registered(key) ?: return "offline"
            if (!reg) continue                                   // not enrolled: the Flipper never signs it
            any = true
            when (val f = src.fetch(key)) {
                HeadFetch.Unreachable -> return "offline"
                HeadFetch.Legacy -> on = false
                is HeadFetch.Active -> entries.add(Entry(g, f.head))
            }
        }
        if (!any) return "no enrolled sensors"
        if (!on) entries.clear()
        val syncOn = synchronized(this) { syncOnSeen[devId] } ?: false
        if (!on && !syncOn && !reanchor) return "chain does not enforce device heads yet: nothing to send"
        val flags = (if (on) F_ON else 0) or (if (reanchor) F_REANCHOR else 0)
        val bytes = headFile(flags, unhex(devId), entries)
        val t = connect(ctx, devId) ?: return "no link to the Flipper (plug in USB or pair over Bluetooth)"
        return try {
            val ok = withTimeout(30_000) { RpcSession(t).writeFile(IN_PATH, bytes) { } }
            if (ok) "sent ${entries.size} head(s), flags $flags" else "the Flipper refused the write"
        } catch (e: Exception) {
            "write failed: ${e.message ?: e.javaClass.simpleName}"
        } finally { runCatching { t.close() } }
    }

    private suspend fun connect(ctx: Context, devId: String): FlipperTransport? = runCatching {
        val usb = ctx.getSystemService(UsbManager::class.java)
        UsbFlipper.find(ctx)?.let { d -> if (usb?.hasPermission(d) == true) return@runCatching UsbFlipper.open(ctx, d) }
        val bonded = BleFlipper.bonded(ctx)
        val want = synchronized(this) { beaconAddr[devId] }?.let { mainAddress(it) }
        val b = bonded.firstOrNull { want != null && it.address.equals(want, ignoreCase = true) } ?: bonded.singleOrNull()
            ?: return@runCatching null
        BleFlipper.open(ctx, b)
    }.onFailure { Log.w(TAG, "connect: ${it.message}") }.getOrNull()

    fun hex(b: ByteArray): String = b.joinToString("") { "%02x".format(it) }
    fun unhex(s: String): ByteArray = s.chunked(2).map { it.toInt(16).toByte() }.toByteArray()
}

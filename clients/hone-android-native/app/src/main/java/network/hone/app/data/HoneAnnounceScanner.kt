package network.hone.app.data

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothManager
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import org.bouncycastle.crypto.params.Ed25519PublicKeyParameters
import org.bouncycastle.crypto.signers.Ed25519Signer
import org.json.JSONObject
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** A HONE device heard over BLE (e.g. the Flipper), from its signed announce. */
data class HoneDevice(
    val deviceId: String,          // first 8 bytes of the pubkey, hex
    val pubkey: String,
    val owner: String,             // as claimed in the announce
    val signatureOk: Boolean,
    val chainOwnerOk: Boolean?,    // null = not checked yet / no node reachable
    val batteryPct: Int,
    val caps: List<String>,
    val extPower: Boolean,
    val paused: Boolean,
    val counter: Long,
    val newestReading: Long,
    val rssi: Int,
    val lastSeenMs: Long,
    val relayed: Int = 0,          // batch commits this phone submitted to the chain
    val queued: Int = 0,           // heard but not yet accepted (no node reachable, etc.)
)

/**
 * Listens for HONE announces: BLE manufacturer data, company 0xFFFF, "HA", seq,
 * idx<<4|total, then up to 23 bytes of a signed v2 frame (type 8). Fragments with
 * the same (address, seq) are joined, the ed25519 signature is checked against the
 * pubkey inside the body, and the claimed owner is checked against the chain: the
 * device's sensors must be enrolled to that owner.
 */
object HoneAnnounceScanner {
    private const val TAG = "HoneAnnounce"
    private const val COMPANY = 0xFFFF
    private val CAPS = listOf("subghz", "nfc", "rfid", "ibutton", "ir", "wifi")

    private val _devices = MutableStateFlow<Map<String, HoneDevice>>(emptyMap())
    val devices: StateFlow<Map<String, HoneDevice>> = _devices.asStateFlow()

    private val frags = HashMap<String, HashMap<Int, ByteArray>>()
    private val done = LinkedHashSet<String>()
    private val chainChecked = HashMap<String, Pair<Boolean?, Long>>() // deviceId -> (ok, at)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    @Volatile private var running = false

    fun hasPermission(ctx: Context): Boolean =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S)
            ctx.checkSelfPermission(Manifest.permission.BLUETOOTH_SCAN) == PackageManager.PERMISSION_GRANTED
        else
            ctx.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

    @SuppressLint("MissingPermission")
    @Synchronized
    fun start(ctx: Context) {
        if (running || !hasPermission(ctx)) return
        if (appCtx == null) loadPending(ctx)
        val scanner = ctx.getSystemService(BluetoothManager::class.java)?.adapter?.bluetoothLeScanner
        if (scanner == null) { Log.w(TAG, "no BLE scanner (Bluetooth off?)"); return }
        // 'HA' = announce, 'HC' = signed batch commit. One exact 2-byte filter each:
        // a 1-byte prefix filter is silently never matched on some phones (moto g).
        fun f(k: Char) = ScanFilter.Builder()
            .setManufacturerData(COMPANY, byteArrayOf('H'.code.toByte(), k.code.toByte()),
                byteArrayOf(0xFF.toByte(), 0xFF.toByte()))
            .build()
        val settings = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_BALANCED)
            .build()
        runCatching { scanner.startScan(listOf(f('A'), f('C')), settings, callback) }
            .onSuccess { running = true; Log.i(TAG, "scanning for HONE announces") }
            .onFailure { Log.e(TAG, "startScan failed", it) }
    }

    private val callback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) {
            val md = result.scanRecord?.getManufacturerSpecificData(COMPANY) ?: return
            if (md.size < 5 || md[0] != 'H'.code.toByte()) return
            if (md[1] == 'C'.code.toByte()) { onCommitFragment(result, md); return }
            if (md[1] != 'A'.code.toByte()) return
            val seq = md[2].toInt() and 0xFF
            val idx = (md[3].toInt() and 0xFF) shr 4
            val total = md[3].toInt() and 0x0F
            if (total == 0 || idx >= total) return
            val key = "${result.device.address}/$seq"
            synchronized(this@HoneAnnounceScanner) {
                if (key in done) { touch(result); return }
                val m = frags.getOrPut(key) { HashMap() }
                m[idx] = md.copyOfRange(4, md.size)
                if (m.size < total) return
                done.add(key); frags.remove(key)
                while (done.size > 256) done.remove(done.first())
                if (frags.size > 256) frags.clear()
                val frame = (0 until total).fold(ByteArray(0)) { acc, i -> acc + m.getValue(i) }
                parse(frame, result.rssi)?.let {
                    addrToId[result.device.address] = it.deviceId
                    publish(it)
                }
            }
        }
        override fun onScanFailed(errorCode: Int) {
            Log.e(TAG, "scan failed: $errorCode"); running = false
        }
    }

    private val addrToId = HashMap<String, String>()

    // ── Batch commits ('HC'): the Flipper signs the chain's SENSOR_DATA_COMMIT
    // message itself; this phone only carries it to a node, so it can do that from
    // anywhere with a connection. Queue survives restarts (relay_queue.json).
    private val GROUPS = listOf("subghz", "wifi", "tags", "ir")
    private val GROUP_TYPE = listOf("continuous", "continuous", "event", "event")
    private val commitFrags = HashMap<String, HashMap<Int, ByteArray>>()
    private val commitDone = LinkedHashSet<String>()
    private val seenBatches = LinkedHashSet<String>()       // batch hashes handled
    private val pending = ArrayList<JSONObject>()            // signed commits awaiting a node
    private val relayedBy = HashMap<String, Int>()
    @Volatile private var appCtx: Context? = null
    @Volatile private var account: String = ""

    private fun onCommitFragment(result: ScanResult, md: ByteArray) {
        val seq = md[2].toInt() and 0xFF
        val idx = (md[3].toInt() and 0xFF) shr 4
        val total = md[3].toInt() and 0x0F
        if (total == 0 || idx >= total) return
        val key = "${result.device.address}/$seq"
        val data: ByteArray
        synchronized(this) {
            if (key in commitDone) return
            val m = commitFrags.getOrPut(key) { HashMap() }
            m[idx] = md.copyOfRange(4, md.size)
            if (m.size < total) return
            commitDone.add(key); commitFrags.remove(key)
            while (commitDone.size > 512) commitDone.remove(commitDone.first())
            if (commitFrags.size > 256) commitFrags.clear()
            data = (0 until total).fold(ByteArray(0)) { acc, i -> acc + m.getValue(i) }
        }
        if (data.size != 107) return
        val devId = data.copyOfRange(0, 8).hex()
        val group = data[8].toInt() and 0xFF
        val count = (data[9].toInt() and 0xFF) or ((data[10].toInt() and 0xFF) shl 8)
        val hash = data.copyOfRange(11, 43).hex()
        val sig = data.copyOfRange(43, 107)
        if (group >= GROUPS.size) return
        synchronized(this) { if (hash in seenBatches) return }
        val heard = _devices.value[devId]?.takeIf { it.signatureOk && it.owner.isNotEmpty() }
        if (heard == null) {
            // Announce not caught yet: take the device key + owner from the chain instead.
            scope.launch {
                val known = chainKey(devId) ?: return@launch
                verifyAndQueue(devId, group, count, hash, sig, known.first, known.second)
            }
            return
        }
        verifyAndQueue(devId, group, count, hash, sig, heard.pubkey, heard.owner)
    }

    private val chainKeys = HashMap<String, Pair<String, String>>() // devId -> (pubkey, owner)

    private fun chainKey(devId: String): Pair<String, String>? {
        chainKeys[devId]?.let { return it }
        val (code, body) = ChainHttp.getWithCode("/api/sensor/flipper-$devId-subghz") ?: return null
        if (code != 200 || body == null) return null
        val j = runCatching { JSONObject(body) }.getOrNull() ?: return null
        val pk = j.optString("device_pubkey", "")
        val owner = j.optString("owner", "")
        if (pk.length != 64 || !pk.startsWith(devId) || owner.isEmpty()) return null
        return (pk to owner).also { chainKeys[devId] = it }
    }

    private fun verifyAndQueue(
        devId: String, group: Int, count: Int, hash: String, sig: ByteArray,
        pubkey: String, owner: String,
    ) {
        val sensorId = "flipper-$devId-${GROUPS[group]}"
        val msg = "{\"chain_id\":\"hone\",\"type\":\"SENSOR_DATA_COMMIT\"," +
            "\"sensor_id\":\"$sensorId\",\"owner\":\"${owner}\",\"batch_hash\":\"$hash\"," +
            "\"reading_count\":$count,\"sensor_type\":\"${GROUP_TYPE[group]}\",\"signed_by\":\"${owner}\"}"
        val ok = runCatching {
            Ed25519Signer().run {
                init(false, Ed25519PublicKeyParameters(pubkey.unhex(), 0))
                val b = msg.toByteArray()
                update(b, 0, b.size)
                verifySignature(sig)
            }
        }.getOrDefault(false)
        if (!ok) { Log.w(TAG, "commit from $devId: bad signature, dropped"); return }
        val body = JSONObject()
            .put("sensor_id", sensorId).put("owner", owner).put("batch_hash", hash)
            .put("reading_count", count).put("sensor_type", GROUP_TYPE[group])
            .put("signature", sig.hex())
        if (account.isNotEmpty()) body.put("gateway_account", account)
        synchronized(this) {
            if (!seenBatches.add(hash)) return
            while (seenBatches.size > 4096) seenBatches.remove(seenBatches.first())
            pending.add(body)
            savePending()
        }
        Log.i(TAG, "commit $sensorId x$count heard, queued")
        updateCounts(devId)
        scope.launch { flush() }
    }

    /** Submit queued commits. 400 = rejected for good (e.g. already committed): drop it. */
    private fun flush() {
        val batch = synchronized(this) { ArrayList(pending) }
        for (b in batch) {
            val res = ChainHttp.postJson("/api/sensor/commit", b.toString()) ?: break // offline
            val devId = b.optString("sensor_id").split("-").getOrNull(1) ?: ""
            if (res.first in 200..299 || res.first in 400..499) {
                synchronized(this) { pending.removeAll { it.optString("batch_hash") == b.optString("batch_hash") }; savePending() }
                if (res.first in 200..299) relayedBy[devId] = (relayedBy[devId] ?: 0) + 1
                else Log.w(TAG, "commit ${b.optString("sensor_id")} refused ${res.first}: ${res.second.take(160)}")
                updateCounts(devId)
            }
        }
    }

    private fun updateCounts(devId: String) {
        val q = synchronized(this) { pending.count { it.optString("sensor_id").contains(devId) } }
        _devices.value[devId]?.let {
            _devices.value = _devices.value + (devId to it.copy(relayed = relayedBy[devId] ?: 0, queued = q))
        }
    }

    private fun queueFile() = appCtx?.filesDir?.resolve("hone-node/relay_queue.json")

    private fun savePending() {
        runCatching { queueFile()?.writeText(org.json.JSONArray(pending).toString()) }
    }

    private fun loadPending(ctx: Context) {
        appCtx = ctx.applicationContext
        account = runCatching { ctx.filesDir.resolve("hone-node/witness_account").readText().trim() }.getOrDefault("")
        runCatching {
            val arr = org.json.JSONArray(queueFile()?.readText() ?: return)
            synchronized(this) {
                for (i in 0 until arr.length()) {
                    val o = arr.getJSONObject(i)
                    pending.add(o); seenBatches.add(o.optString("batch_hash"))
                }
            }
        }
        scope.launch {
            while (true) { flush(); kotlinx.coroutines.delay(60_000) }
        }
    }

    /** Refresh rssi / last-seen on every repeat fragment of an already-verified announce. */
    private fun touch(r: ScanResult) {
        val id = addrToId[r.device.address] ?: return
        val d = _devices.value[id] ?: return
        _devices.value = _devices.value + (id to d.copy(rssi = r.rssi, lastSeenMs = System.currentTimeMillis()))
    }

    private fun parse(f: ByteArray, rssi: Int): HoneDevice? {
        if (f.size < 32 + 64 || f[0] != 'H'.code.toByte() || f[1] != '2'.code.toByte()) return null
        val type = f[2].toInt() and 0xFF
        val blen = f[3].toInt() and 0xFF
        if (type != 8 || f.size != 32 + blen + 64 || blen < 56) return null
        val bb = ByteBuffer.wrap(f).order(ByteOrder.LITTLE_ENDIAN)
        val devId = f.copyOfRange(4, 12)
        val counter = bb.getInt(12).toLong() and 0xFFFFFFFFL
        val body = f.copyOfRange(32, 32 + blen)
        val sig = f.copyOfRange(32 + blen, f.size)
        val pk = body.copyOfRange(0, 32)
        val b = ByteBuffer.wrap(body).order(ByteOrder.LITTLE_ENDIAN)
        val caps = b.getShort(32).toInt() and 0xFFFF
        val newest = b.getInt(34).toLong() and 0xFFFFFFFFL
        val batt = body[38].toInt() and 0xFF
        val flags = body[39].toInt() and 0xFF
        val owner = String(body.copyOfRange(40, 56), Charsets.UTF_8).trimEnd('\u0000').trim()
        val sigOk = runCatching {
            Ed25519Signer().run {
                init(false, Ed25519PublicKeyParameters(pk, 0))
                update(f, 0, 32 + blen)
                verifySignature(sig)
            }
        }.getOrDefault(false) && devId.contentEquals(pk.copyOfRange(0, 8))
        val id = devId.hex()
        return HoneDevice(
            deviceId = id, pubkey = pk.hex(), owner = owner, signatureOk = sigOk,
            chainOwnerOk = chainChecked[id]?.first,
            batteryPct = batt, caps = CAPS.filterIndexed { i, _ -> (caps shr i) and 1 == 1 },
            extPower = flags and 1 != 0, paused = flags and 2 != 0,
            counter = counter, newestReading = newest, rssi = rssi,
            lastSeenMs = System.currentTimeMillis(),
        )
    }

    private fun publish(d0: HoneDevice) {
        val prev = _devices.value[d0.deviceId]
        val d = d0.copy(relayed = relayedBy[d0.deviceId] ?: 0, queued = prev?.queued ?: 0)
        _devices.value = _devices.value + (d.deviceId to d)
        Log.i(TAG, "announce ${d.deviceId} owner=${d.owner} sig=${d.signatureOk} batt=${d.batteryPct}%")
        val last = chainChecked[d.deviceId]
        if (d.signatureOk && (last == null || System.currentTimeMillis() - last.second > 300_000)) {
            chainChecked[d.deviceId] = (last?.first) to System.currentTimeMillis()
            scope.launch {
                val ok = chainOwner(d)
                chainChecked[d.deviceId] = ok to System.currentTimeMillis()
                _devices.value[d.deviceId]?.let {
                    _devices.value = _devices.value + (d.deviceId to it.copy(chainOwnerOk = ok))
                }
            }
        }
    }

    /** The owner claim holds if one of the device's sensors is enrolled on chain to that
     *  owner (and, when the node reports it, to this exact device key). */
    private fun chainOwner(d: HoneDevice): Boolean? {
        var reached = false
        for (suffix in listOf("subghz", "tags", "ir", "wifi")) {
            val (code, body) = ChainHttp.getWithCode("/api/sensor/flipper-${d.deviceId}-$suffix") ?: continue
            reached = true
            if (code == 404 || body == null) continue
            val j = runCatching { JSONObject(body) }.getOrNull() ?: continue
            val owner = j.optString("owner")
            val devKey = j.optString("device_pubkey", "")
            if (owner == d.owner && (devKey.isEmpty() || devKey.equals(d.pubkey, ignoreCase = true))) return true
            if (owner.isNotEmpty() && owner != d.owner) return false
        }
        return if (reached) false else null
    }

    private fun ByteArray.hex(): String = joinToString("") { "%02x".format(it) }
    private fun String.unhex(): ByteArray = chunked(2).map { it.toInt(16).toByte() }.toByteArray()
}

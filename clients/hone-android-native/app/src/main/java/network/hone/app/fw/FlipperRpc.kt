package network.hone.app.fw

import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.withTimeout
import java.io.Closeable

/**
 * Flipper RPC (protobuf over the BLE serial service or USB CDC), the protocol the official
 * Flipper mobile app uses. Field numbers from assets/protobuf in the HONE firmware tree:
 * Main{command_id=1, command_status=2, has_next=3, ... system_ping_request=5, storage_write_request=11,
 * storage_mkdir_request=13, storage_md5sum_request=14 / response=15, storage_info_request=28 / response=29,
 * system_reboot_request=31, system_device_info_request=32 / response=33, system_update_request=41 /
 * response=46, system_power_info_request=44 / response=45}.
 */
object Rpc {
    const val OK = 0L

    data class Resp(val id: Long, val status: Long, val hasNext: Boolean, val field: Int?, val body: ByteArray?)

    fun main(id: Long, field: Int, body: Pb.W, hasNext: Boolean = false): ByteArray =
        Pb.delimited(Pb.W().u(1, id).b(3, hasNext).msg(field, body).toBytes())

    fun ping(id: Long) = main(id, 5, Pb.W().bytes(1, byteArrayOf(1, 2, 3)))
    fun deviceInfo(id: Long) = main(id, 32, Pb.W())
    fun powerInfo(id: Long) = main(id, 44, Pb.W())
    fun storageInfo(id: Long, path: String) = main(id, 28, Pb.W().str(1, path))
    fun mkdir(id: Long, path: String) = main(id, 13, Pb.W().str(1, path))
    fun md5(id: Long, path: String) = main(id, 14, Pb.W().str(1, path))
    /** One chunk of a file write; every chunk but the last has has_next = true (same command id). */
    fun write(id: Long, path: String, chunk: ByteArray, hasNext: Boolean) =
        main(id, 11, Pb.W().str(1, path).msg(2, Pb.W().bytes(4, chunk)), hasNext)
    fun update(id: Long, manifestPath: String) = main(id, 41, Pb.W().str(1, manifestPath))
    /** RebootRequest mode UPDATE = 2. */
    fun rebootToUpdate(id: Long) = main(id, 31, Pb.W().u(1, 2))

    fun parse(m: ByteArray): Resp {
        val f = Pb.read(m)
        val content = f.keys.firstOrNull { it > 3 }
        return Resp((f[1]?.first() as? Long) ?: 0, (f[2]?.first() as? Long) ?: 0, ((f[3]?.first() as? Long) ?: 0) != 0L,
            content, content?.let { f[it]?.first() as? ByteArray })
    }

    /** key/value pairs from DeviceInfo / PowerInfo responses (field 1 key, 2 value). */
    fun kv(body: ByteArray?): Pair<String, String>? = body?.let { b ->
        val f = Pb.read(b)
        ((f[1]?.first() as? ByteArray)?.toString(Charsets.UTF_8) ?: return null) to ((f[2]?.first() as? ByteArray)?.toString(Charsets.UTF_8) ?: "")
    }
}

/** A byte pipe to a Flipper in RPC mode (BLE serial or USB CDC). */
interface FlipperTransport : Closeable {
    val name: String
    /** Largest write chunk this link handles well. */
    val chunk: Int
    suspend fun send(b: ByteArray)
    /** Incoming bytes, in order. */
    val incoming: Channel<ByteArray>
}

/** Request/response over a transport; one command at a time. */
class RpcSession(private val t: FlipperTransport) {
    private var nextId = 1L
    private var buf = ByteArray(0)

    private suspend fun recv(timeoutMs: Long): Rpc.Resp = withTimeout(timeoutMs) {
        while (true) {
            val (msgs, _) = Pb.split(buf)
            if (msgs.isNotEmpty()) { buf = buf.copyOfRange(Pb.delimited(msgs[0]).size, buf.size); return@withTimeout Rpc.parse(msgs[0]) }
            buf += t.incoming.receive()
        }
        @Suppress("UNREACHABLE_CODE")
        throw IllegalStateException()
    }

    /** Sends one request; collects all responses for its id (has_next chains). */
    suspend fun call(build: (Long) -> ByteArray, timeoutMs: Long = 15_000): List<Rpc.Resp> {
        val id = nextId++
        t.send(build(id))
        val out = ArrayList<Rpc.Resp>()
        while (true) {
            val r = recv(timeoutMs)
            if (r.id != id) continue
            out.add(r)
            if (!r.hasNext) return out
        }
    }

    suspend fun ping() = call({ Rpc.ping(it) }).last().status == Rpc.OK
    suspend fun deviceInfo(): Map<String, String> = call({ Rpc.deviceInfo(it) }).mapNotNull { Rpc.kv(it.body) }.toMap()
    suspend fun powerInfo(): Map<String, String> = call({ Rpc.powerInfo(it) }).mapNotNull { Rpc.kv(it.body) }.toMap()
    suspend fun freeSpace(path: String): Long? = call({ Rpc.storageInfo(it, path) }).last().let { r ->
        if (r.status != Rpc.OK) null else (Pb.read(r.body ?: ByteArray(0))[2]?.first() as? Long) ?: 0L
    }
    /** Creates a folder; "already exists" is fine, so the status is not checked (the writes that follow are). */
    suspend fun mkdir(path: String) { call({ Rpc.mkdir(it, path) }) }
    suspend fun md5(path: String): String? = call({ Rpc.md5(it, path) }).last().let { r ->
        if (r.status != Rpc.OK) null else (Pb.read(r.body ?: ByteArray(0))[1]?.first() as? ByteArray)?.toString(Charsets.UTF_8)
    }

    /** Writes a whole file in chunks under one command id; [progress] gets bytes sent. */
    suspend fun writeFile(path: String, data: ByteArray, progress: (Int) -> Unit): Boolean {
        val id = nextId++
        var off = 0
        if (data.isEmpty()) { t.send(Rpc.write(id, path, ByteArray(0), false)) }
        while (off < data.size) {
            val end = minOf(data.size, off + t.chunk)
            t.send(Rpc.write(id, path, data.copyOfRange(off, end), end < data.size))
            off = end; progress(off)
        }
        while (true) { val r = recv(60_000); if (r.id == id) return r.status == Rpc.OK }
    }

    /** SystemUpdateRequest; returns the UpdateResultCode (0 = OK). */
    suspend fun update(manifestPath: String): Long = call({ Rpc.update(it, manifestPath) }).last().let { r ->
        if (r.status != Rpc.OK) -1 else (Pb.read(r.body ?: ByteArray(0))[1]?.first() as? Long) ?: 0L
    }
    suspend fun rebootToUpdate() = runCatching { t.send(Rpc.rebootToUpdate(nextId++)) }
}

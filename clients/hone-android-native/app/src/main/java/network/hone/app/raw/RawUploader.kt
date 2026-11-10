package network.hone.app.raw

import android.content.Context
import android.net.ConnectivityManager
import android.util.Base64
import network.hone.app.telemetry.Canonical
import network.hone.app.telemetry.Telemetry
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Uploads queued raw window blobs to honemesh.net/api/raw/upload, signed by this device's seat
 * key (owner = account@<seat pubkey>). On Wi-Fi (unmetered) whenever due; on mobile data only
 * when the oldest blob has waited [MOBILE_AFTER_MS]. Nothing is ever deleted unless the ingest
 * has it; blobs the ingest refuses for good move to raw-queue/refused (kept, reported).
 */
object RawUploader {
    const val URL_UPLOAD = "https://honemesh.net/api/raw/upload"
    private const val EVERY_MS = 15 * 60_000L
    private const val MOBILE_AFTER_MS = 6 * 3600_000L
    private const val PER_ROUND = 40
    private val busy = AtomicBoolean(false)
    @Volatile private var lastMs = 0L

    fun maybeFlush(ctx: Context) {
        val now = System.currentTimeMillis()
        if (now - lastMs < EVERY_MS || !busy.compareAndSet(false, true)) return
        lastMs = now
        val c = ctx.applicationContext
        Thread({ try { runCatching { flush(c) }.onFailure { Telemetry.error(c, "raw upload", it) } } finally { busy.set(false) } }, "hone-raw").start()
    }

    private fun unmetered(c: Context) = runCatching { c.getSystemService(ConnectivityManager::class.java)?.isActiveNetworkMetered == false }.getOrDefault(false)

    fun flush(c: Context) {
        val q = RawRecorder.qdir(c)
        val metas = q.listFiles { f -> f.name.endsWith(".json") }?.sortedBy { it.name } ?: return
        if (metas.isEmpty()) return
        if (!unmetered(c) && System.currentTimeMillis() - metas.first().lastModified() < MOBILE_AFTER_MS) {
            Telemetry.skipped(c, "raw", "waiting for Wi-Fi (${metas.size} queued)"); return
        }
        val account = Telemetry.seatAccount(c) ?: run { Telemetry.skipped(c, "raw", "no seat account"); return }
        val seed = Telemetry.seatSeedForChain(c)
        val owner = "$account@${Telemetry.seatPub(c)}"
        for (mf in metas.take(PER_ROUND)) {
            val gz = File(mf.path.removeSuffix(".json") + ".gz")
            if (!gz.exists()) { mf.delete(); continue }
            val info = JSONObject(mf.readText())
            val body = gz.readBytes()
            val meta = Canonical.encode(mapOf("v" to 1, "owner" to owner, "device" to Telemetry.deviceId(c),
                "window" to info.getLong("window"), "cell" to info.optString("cell").takeIf { it.isNotEmpty() && it != "null" },
                "sha256" to info.getString("sha256"), "size" to body.size, "ts" to System.currentTimeMillis() / 1000, "kind" to "phone"))
            val sig = Canonical.hex(Canonical.sign(seed, meta))
            val (code, resp) = post(meta, sig, body)
            when {
                code == 200 -> { keepOwn(c, gz); mf.delete(); Telemetry.upload(c, "raw", true) }
                code == 400 || code == 403 || code == 413 -> {
                    val r = File(q, "refused"); r.mkdirs()
                    gz.renameTo(File(r, gz.name)); mf.renameTo(File(r, mf.name))
                    Telemetry.upload(c, "raw", false, "refused $code ${resp.take(100)}")
                }
                else -> { Telemetry.upload(c, "raw", false, if (code == 0) "offline" else "HTTP $code ${resp.take(80)}"); return }
            }
        }
    }

    private fun post(meta: String, sig: String, body: ByteArray): Pair<Int, String> = runCatching {
        val conn = URL(URL_UPLOAD).openConnection() as HttpURLConnection
        try {
            conn.requestMethod = "POST"; conn.doOutput = true
            conn.connectTimeout = 15_000; conn.readTimeout = 60_000
            conn.setRequestProperty("content-type", "application/gzip")
            conn.setRequestProperty("x-hone-meta", Base64.encodeToString(meta.toByteArray(), Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING))
            conn.setRequestProperty("x-hone-sig", sig)
            conn.setFixedLengthStreamingMode(body.size)
            conn.outputStream.use { it.write(body) }
            val code = conn.responseCode
            val text = (if (code < 400) conn.inputStream else conn.errorStream)?.bufferedReader()?.readText() ?: ""
            code to text
        } finally { conn.disconnect() }
    }.getOrDefault(0 to "")

    /** Owner self-storage: this phone keeps its own uploaded blobs (served on request), up to [KEEP_BYTES]. */
    const val KEEP_BYTES = 128L shl 20
    fun storeDir(c: Context) = File(c.filesDir, "hone-node/raw-store")
    private fun keepOwn(c: Context, gz: File) {
        val d = storeDir(c); d.mkdirs()
        if (!gz.renameTo(File(d, gz.name))) { gz.delete(); return }
        val files = d.listFiles { f -> f.name.endsWith(".gz") }?.sortedBy { it.lastModified() } ?: return
        var total = files.sumOf { it.length() }
        for (f in files) { if (total <= KEEP_BYTES) break; total -= f.length(); f.delete() }
    }

    fun queued(c: Context): Int = RawRecorder.qdir(c).listFiles { f -> f.name.endsWith(".json") }?.size ?: 0
}

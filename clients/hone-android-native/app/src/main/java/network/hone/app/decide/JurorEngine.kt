package network.hone.app.decide

import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.BatteryManager
import android.os.Build
import android.os.PowerManager
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest

/** What the Node screen can show about the decide juror. */
data class JurorStatus(
    val enabled: Boolean = false,
    val eligible: Boolean = false,
    val reason: String = "off",
    val modelReady: Boolean = false,
    val running: Boolean = false,
    val votesPerEpoch: Int = 0,
)

/**
 * HONE decide juror on the phone. It runs the hone-decide sidecar (the same Rust binary as
 * nebra, bundled as libhone_decide.so) as a child process on 127.0.0.1, so votes, seals and
 * reveals follow docs/JURY.md exactly and the model_hash matches every other node running the
 * same files.
 *
 * Eligibility (re-checked every 30 s): charging, on unmetered Wi-Fi, and thermal status below
 * MODERATE. When ineligible the sidecar keeps running but reports /capacity = 0 and refuses
 * /vote (eligible file contains "0"). Thermal MODERATE or worse also backs off.
 *
 * The model is downloaded over Wi-Fi from `model_url_base` and verified file by file against
 * the sha256 values in the settings; it is never shipped inside the APK.
 *
 * Settings: files/hone-decide/juror.json (written by the operator, e.g. adb run-as):
 *   {"enabled": true, "model_url_base": "http://127.0.0.1:8099/smollm2-360m-gen-q8/",
 *    "files": {"model-q8_0.gguf": "<sha256>", "config.json": "...", "decide.json": "...",
 *              "tokenizer.json": "...", "head.safetensors": "...", "thresholds.json": "..."},
 *    "port": 4251, "threads": 4}
 * Committing votes on chain (DECIDE_COMMIT with the phone's clock-seat key) is done by the node
 * integration once its entry types exist; this engine only owns the sidecar and its gate.
 */
object JurorEngine {
    private const val TAG = "HoneJuror"
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var job: Job? = null
    private var proc: Process? = null
    private val _status = MutableStateFlow(JurorStatus())
    val status: StateFlow<JurorStatus> = _status.asStateFlow()

    fun start(ctx: Context) {
        if (job?.isActive == true) return
        job = scope.launch { loop(ctx.applicationContext) }
    }

    fun stop() {
        job?.cancel(); job = null
        proc?.destroy(); proc = null
        _status.value = JurorStatus()
    }

    private fun dir(ctx: Context) = File(ctx.filesDir, "hone-decide").apply { mkdirs() }

    private suspend fun loop(ctx: Context) {
        var backoffMs = 5_000L
        while (scope.isActive) {
            val cfg = runCatching { JSONObject(File(dir(ctx), "juror.json").readText()) }.getOrNull()
            if (cfg == null || !cfg.optBoolean("enabled", false)) {
                proc?.destroy(); proc = null
                _status.value = JurorStatus(reason = "disabled (no files/hone-decide/juror.json)")
                delay(60_000); continue
            }
            val (ok, reason) = eligibility(ctx)
            File(dir(ctx), "eligible").writeText(if (ok) "1" else "0")
            val modelDir = File(dir(ctx), "model")
            val ready = (ok && ensureModel(cfg, modelDir)) || modelPresent(cfg, modelDir)
            if (ready && proc?.isAlive != true) {
                proc = runCatching { launch(ctx, cfg, modelDir) }.onFailure { Log.w(TAG, "launch failed", it) }.getOrNull()
                if (proc == null) { delay(backoffMs); backoffMs = (backoffMs * 2).coerceAtMost(600_000); continue }
                backoffMs = 5_000L
            }
            val vpe = if (proc?.isAlive == true) capacity(cfg.optInt("port", 4251)) else 0
            _status.value = JurorStatus(true, ok, reason, ready, proc?.isAlive == true, vpe)
            delay(30_000)
        }
    }

    /** Charging + unmetered Wi-Fi + thermal below MODERATE. */
    fun eligibility(ctx: Context): Pair<Boolean, String> {
        val bat = ctx.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val plugged = (bat?.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) ?: 0) != 0
        if (!plugged) return false to "not charging"
        val cm = ctx.getSystemService(ConnectivityManager::class.java)
        val caps = cm?.getNetworkCapabilities(cm.activeNetwork)
        val wifi = caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true &&
            caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
        if (!wifi) return false to "not on unmetered Wi-Fi"
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val t = ctx.getSystemService(PowerManager::class.java)?.currentThermalStatus ?: 0
            if (t >= PowerManager.THERMAL_STATUS_MODERATE) return false to "thermal throttling (status $t)"
        }
        return true to "eligible"
    }

    private fun modelPresent(cfg: JSONObject, dir: File): Boolean {
        val files = cfg.optJSONObject("files") ?: return false
        return files.keys().asSequence().all { File(dir, it).exists() }
    }

    /** Download missing or mismatched files and verify sha256; returns true when all verify. */
    private fun ensureModel(cfg: JSONObject, dir: File): Boolean {
        val files = cfg.optJSONObject("files") ?: return false
        val base = cfg.optString("model_url_base").let { if (it.endsWith("/")) it else "$it/" }
        dir.mkdirs()
        for (name in files.keys()) {
            val want = files.getString(name).lowercase()
            val f = File(dir, name)
            if (f.exists() && sha256(f) == want) continue
            val tmp = File(dir, "$name.part")
            runCatching {
                val c = URL(base + name).openConnection() as HttpURLConnection
                c.connectTimeout = 15_000; c.readTimeout = 60_000
                c.inputStream.use { i -> tmp.outputStream().use { o -> i.copyTo(o, 1 shl 16) } }
            }.onFailure { Log.w(TAG, "download $name failed", it); tmp.delete(); return false }
            if (sha256(tmp) != want) { Log.w(TAG, "sha256 mismatch for $name"); tmp.delete(); return false }
            if (!tmp.renameTo(f)) return false
        }
        return true
    }

    private fun sha256(f: File): String {
        val md = MessageDigest.getInstance("SHA-256")
        f.inputStream().use { i -> val b = ByteArray(1 shl 16); while (true) { val k = i.read(b); if (k < 0) break; md.update(b, 0, k) } }
        return md.digest().joinToString("") { "%02x".format(it) }
    }

    private fun launch(ctx: Context, cfg: JSONObject, modelDir: File): Process {
        val exe = File(ctx.applicationInfo.nativeLibraryDir, "libhone_decide.so")
        val d = dir(ctx)
        val pb = ProcessBuilder(exe.absolutePath, "serve", "--model", modelDir.absolutePath,
            "--bind", "127.0.0.1:${cfg.optInt("port", 4251)}", "--max-length", "1024",
            "--votes-dir", File(d, "votes").absolutePath, "--eligible-file", File(d, "eligible").absolutePath,
            "--max-rss-mb", cfg.optInt("max_rss_mb", 700).toString())
        pb.environment()["RAYON_NUM_THREADS"] = cfg.optInt("threads", 4).toString()
        pb.redirectErrorStream(true).redirectOutput(File(d, "sidecar.log"))
        Log.i(TAG, "starting hone-decide sidecar")
        return pb.start()
    }

    private fun capacity(port: Int): Int = runCatching {
        val c = URL("http://127.0.0.1:$port/capacity").openConnection() as HttpURLConnection
        c.connectTimeout = 2_000; c.readTimeout = 120_000
        JSONObject(c.inputStream.bufferedReader().readText()).optInt("votes_per_epoch", 0)
    }.getOrDefault(0)
}

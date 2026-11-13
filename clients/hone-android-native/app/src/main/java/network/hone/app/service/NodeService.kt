package network.hone.app.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import network.hone.app.data.AndroidSensorKeyStore
import network.hone.app.data.DeviceSensorIdentity
import network.hone.app.data.NodeSensorTransport
import network.hone.app.data.SensorCapture
import network.hone.app.data.SensorCommitClient
import network.hone.app.data.SensorNodeSettings
import network.hone.app.data.SensorReadingBatch
import network.hone.app.data.SensorSubmissionCoordinator
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest

/**
 * Foreground service that keeps local sensor capture alive in the background — the piece a
 * webview cannot do (Android kills a webview when it loses focus).
 *
 * Running behaviour:
 *   - captures every sensor the device has (SensorCapture);
 *   - with no operator settings it stays in read-only capture;
 *   - when `files/hone-node/sensor_settings.json` supplies chain id, owner, node URL,
 *     sensor-id prefix and the channels to submit, it commits one signed
 *     SensorDataCommit per channel per 30 s window to the node's HTTP API. Each channel
 *     signs with its own AndroidKeyStore Ed25519 key (never exported); the public keys
 *     are written to `files/hone-node/sensor_pubkeys.json` so the owner can enroll them
 *     once (SensorKeyRegister signed by the owner's posting key, done off-device).
 *
 * Settings example (written by the operator, e.g. via adb run-as):
 *   {"chain_id":"hone","owner":"josh","node_url":"https://honemesh.net/chain",
 *    "sensor_id_prefix":"moto-gpower","sensor_type":"continuous",
 *    "channels":["accel","gyro","mag","baro","light","proximity"]}
 * GNSS is never submitted by default: its value would publish the phone's location.
 *
 * specialUse FGS (not dataSync) so Android 15+ does not force-stop it after ~6h/day.
 * START_STICKY + BootReceiver lets capture recover after a kill or reboot.
 */
class NodeService : Service() {

    private val scope = CoroutineScope(Dispatchers.IO)
    private var loop: Job? = null
    private lateinit var capture: SensorCapture

    private data class Submission(
        val settings: SensorNodeSettings,
        val sensorType: String,
        val channels: Set<String>,
    )

    override fun onCreate() {
        super.onCreate()
        capture = SensorCapture(applicationContext)
        network.hone.app.telemetry.Telemetry.installCrashHandler(applicationContext)
        runCatching { network.hone.app.telemetry.StatusWorker.schedule(applicationContext) }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForegroundCompat(buildNotification("Starting sensor node…"))
        if (network.hone.app.telemetry.Telemetry.serviceStartedMs == 0L)
            network.hone.app.telemetry.Telemetry.serviceStartedMs = System.currentTimeMillis()
        capture.start()
        // Keep listening for HONE device announces while the node runs in the background.
        network.hone.app.data.HoneAnnounceScanner.start(applicationContext)
        // Coverage sensing + rotating-id advert (spec 1a/1b/2a).
        network.hone.app.sightings.SightingsEngine.start(applicationContext)
        // Sound sensing (bird check + ambient noise), only if the user accepted the notice.
        network.hone.app.sound.SoundSensing.start(applicationContext)
        // Bring the witness clock seat back after a reboot or app update.
        if (File(dataDir(), "witness_account").exists()) scope.launch {
            (network.hone.app.data.NodeRepositories.get(applicationContext)
                as? network.hone.app.data.BridgeNodeRepository)?.ensureStarted()
        }
        if (loop?.isActive != true) loop = scope.launch { runLoop() }
        return START_STICKY
    }

    private fun dataDir(): File = File(filesDir, "hone-node").apply { mkdirs() }

    private fun loadSubmission(): Submission? = runCatching {
        val f = File(dataDir(), "sensor_settings.json")
        if (!f.exists()) return null
        val j = JSONObject(f.readText())
        val settings = SensorNodeSettings(
            chainId = j.getString("chain_id"),
            owner = j.getString("owner"),
            nodeUrl = publicNodeUrl(j.getString("node_url")),
            sensorIdPrefix = j.getString("sensor_id_prefix"),
        )
        val chans = j.optJSONArray("channels")
        val set = buildSet { if (chans != null) for (i in 0 until chans.length()) add(chans.getString(i)) }
        Submission(settings, j.optString("sensor_type", "continuous"), set)
    }.onFailure { Log.w(TAG, "sensor_settings.json invalid: ${it.message}") }.getOrNull()

    /** LAN / tailscale node URLs from older settings only work at home: use the public node API. */
    private fun publicNodeUrl(u: String): String {
        val host = runCatching { java.net.URI(u).host }.getOrNull() ?: return network.hone.app.data.ChainHttp.PUBLIC_BASE
        val private = host == "localhost" || host.endsWith(".local") || host.endsWith(".ts.net") ||
            Regex("^(10|127)\\..*|^192\\.168\\..*|^172\\.(1[6-9]|2[0-9]|3[01])\\..*|^100\\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\\..*").matches(host)
        return if (private) network.hone.app.data.ChainHttp.PUBLIC_BASE else u
    }

    /** Public publishing (birds, Flipper frames, device status): runs every loop, with or without chain settings. */
    private fun publishPublic() {
        val c = applicationContext
        runCatching { network.hone.app.home.HomeState.tick(c) }.onFailure { network.hone.app.telemetry.Telemetry.error(c, "home", it) }
        runCatching { network.hone.app.birds.BirdsSync.syncNow(c) }.onFailure { network.hone.app.telemetry.Telemetry.error(c, "birds", it) }
        runCatching { network.hone.app.raw.RawUploader.maybeFlush(c) }.onFailure { network.hone.app.telemetry.Telemetry.error(c, "raw", it) }
        runCatching { network.hone.app.telemetry.CoverageSubmitter.maybeFlush(c) }.onFailure { network.hone.app.telemetry.Telemetry.error(c, "coverage", it) }
        runCatching { network.hone.app.birds.BirdChainSubmitter.maybeFlush(c) }.onFailure { network.hone.app.telemetry.Telemetry.error(c, "bird chain", it) }
        runCatching { network.hone.app.telemetry.FlipperPublisher.maybeFlush(c) }.onFailure { network.hone.app.telemetry.Telemetry.error(c, "flipper", it) }
        runCatching { network.hone.app.telemetry.DeviceStatus.maybeSend(c) }.onFailure { network.hone.app.telemetry.Telemetry.error(c, "status", it) }
    }

    private suspend fun runLoop() {
        val sub = loadSubmission()
        val keystore = if (sub != null && Build.VERSION.SDK_INT >= 33) {
            runCatching { AndroidSensorKeyStore() }
                .onFailure { Log.e(TAG, "AndroidKeyStore unavailable", it) }
                .getOrNull()
        } else null
        val coordinator = sub?.let {
            SensorSubmissionCoordinator(SensorCommitClient(it.settings.chainId, NodeSensorTransport(it.settings.nodeUrl)))
        }
        val identities = HashMap<String, DeviceSensorIdentity>()
        var accepted = 0L
        var rejected = 0L
        var lastError = ""

        while (scope.isActive) {
            delay(COMMIT_INTERVAL_MS)
            publishPublic()
            val avail = capture.availableCount()
            if (sub == null || keystore == null || coordinator == null) {
                updateNotification("$avail sensors · read-only capture")
                continue
            }
            for (ch in capture.channels.value) {
                if (!ch.available || ch.id !in sub.channels) continue
                val n = capture.drainSampleCount(ch.id)
                if (n == 0L) continue
                val sensorId = sub.settings.sensorId(ch.id)
                val identity = identities[sensorId]
                    ?: runCatching { keystore.getOrCreate(sensorId) }
                        .onFailure { Log.e(TAG, "device key for $sensorId failed", it) }
                        .getOrNull()
                        ?.also { identities[sensorId] = it }
                    ?: continue
                val batchHash = sha256Hex("$sensorId|$n|${ch.value}|${System.currentTimeMillis()}")
                val batch = SensorReadingBatch(
                    sensorId = sensorId,
                    owner = sub.settings.owner,
                    batchHash = batchHash,
                    readingCount = n,
                    sensorType = sub.sensorType,
                )
                val resp = runCatching { coordinator.submitWithEnrolledDevice(batch, identity) }
                    .getOrElse { e -> network.hone.app.data.SensorHttpResponse(0, e.message ?: "error") }
                if (resp.accepted) accepted++ else {
                    rejected++
                    lastError = resp.body.take(160)
                    Log.w(TAG, "commit $sensorId rejected (${resp.status}): $lastError")
                }
            }
            if ("sound" in sub.channels) {
                val sensorId = sub.settings.sensorId("sound")
                val identity = identities[sensorId]
                    ?: runCatching { keystore.getOrCreate(sensorId) }.getOrNull()?.also { identities[sensorId] = it }
                if (identity != null) for (r in network.hone.app.sound.SoundSensing.drainReports()) {
                    // Report stays on the phone (files/hone-node/sound); the chain gets its hash.
                    val batch = SensorReadingBatch(
                        sensorId = sensorId,
                        owner = sub.settings.owner,
                        batchHash = network.hone.app.sound.SoundSensing.reportHash(r),
                        readingCount = r.optLong("frames", 1L).coerceAtLeast(1L),
                        sensorType = "acoustic",
                    )
                    val resp = runCatching { coordinator.submitWithEnrolledDevice(batch, identity) }
                        .getOrElse { e -> network.hone.app.data.SensorHttpResponse(0, e.message ?: "error") }
                    if (resp.accepted) accepted++ else { rejected++; lastError = resp.body.take(160) }
                }
            }
            writePubkeys(identities)
            network.hone.app.telemetry.Telemetry.sensorAccepted.set(accepted)
            network.hone.app.telemetry.Telemetry.sensorRejected.set(rejected)
            network.hone.app.telemetry.Telemetry.sensorLastError = lastError
            updateNotification("$avail sensors · $accepted accepted · $rejected rejected")
        }
    }

    private fun writePubkeys(ids: Map<String, DeviceSensorIdentity>) {
        if (ids.isEmpty()) return
        val j = JSONObject()
        ids.toSortedMap().forEach { (k, v) -> j.put(k, v.publicKeyHex) }
        runCatching { File(dataDir(), "sensor_pubkeys.json").writeText(j.toString()) }
    }

    private fun sha256Hex(s: String): String =
        MessageDigest.getInstance("SHA-256").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }

    override fun onDestroy() {
        network.hone.app.sightings.SightingsEngine.stop()
        network.hone.app.sound.SoundSensing.stop()
        capture.stop()
        loop?.cancel()
        scope.cancel()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun startForegroundCompat(n: Notification) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            // Add the location type only when it is allowed; otherwise run without it
            // (readings are then kept unlocated) rather than failing to start.
            val loc = checkSelfPermission(android.Manifest.permission.ACCESS_FINE_LOCATION) ==
                android.content.pm.PackageManager.PERMISSION_GRANTED
            val mic = network.hone.app.sound.SoundSensing.isEnabled(this) &&
                network.hone.app.sound.SoundSensing.hasMic(this)
            val base = ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
            val locT = if (loc) ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION else 0
            val micT = if (mic) ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else 0
            // The microphone type can only start while the app is visible (Android 14+); from a
            // boot or background start it is refused, so fall back step by step.
            val ok = listOf(base or locT or micT, base or locT, base).distinct().any { t ->
                runCatching { startForeground(NOTIF_ID, n, t) }
                    .onFailure { Log.w(TAG, "FGS types $t refused (${it.message})") }
                    .onSuccess { fgsTypes = t; if (micT != 0 && t and micT == 0) Log.w(TAG, "running without the microphone type") }
                    .isSuccess
            }
            if (!ok) Log.e(TAG, "foreground start refused for all type sets")
        } else {
            startForeground(NOTIF_ID, n)
        }
    }

    private fun buildNotification(status: String): Notification =
        NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("HONE sensor node")
            .setContentText(status)
            .setSmallIcon(android.R.drawable.ic_menu_compass)
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()

    private fun updateNotification(status: String) {
        getSystemService(NotificationManager::class.java).notify(NOTIF_ID, buildNotification(status))
    }

    companion object {
        private const val TAG = "HoneSensors"
        private const val CHANNEL_ID = "hone_node"
        private const val NOTIF_ID = 1001
        private const val COMMIT_INTERVAL_MS = 30_000L
        /** Foreground service types the system accepted last (0 = not started). */
        @Volatile var fgsTypes: Int = 0

        fun ensureChannel(ctx: Context) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                val channel = NotificationChannel(
                    CHANNEL_ID, "HONE node", NotificationManager.IMPORTANCE_LOW,
                ).apply { description = "Keeps your sensor node submitting in the background." }
                ctx.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
            }
        }

        fun start(ctx: Context) {
            ensureChannel(ctx)
            val intent = Intent(ctx, NodeService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) ctx.startForegroundService(intent)
            else ctx.startService(intent)
        }

        fun stop(ctx: Context) { ctx.stopService(Intent(ctx, NodeService::class.java)) }
    }
}

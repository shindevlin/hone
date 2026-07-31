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
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import network.hone.app.data.SensorCapture

/**
 * Foreground service that keeps local sensor capture alive in the background — the piece a
 * webview cannot do (Android kills a webview when it loses focus).
 *
 * Running behaviour:
 *   - captures every sensor the device has (SensorCapture),
 *   - captures locally in read-only mode until explicit account/node configuration and
 *     device enrollment are supplied by the app state layer,
 *   - reports live status in the ongoing notification.
 *
 * specialUse FGS (not dataSync) so Android 15+ does not force-stop it after ~6h/day.
 * START_STICKY + BootReceiver lets capture recover after a kill or reboot. This checked-in
 * service does not submit to a node: submission requires explicit settings, an enrolled
 * AndroidKeyStore identity, and a SensorSubmissionCoordinator supplied by app state.
 */
class NodeService : Service() {

    private val scope = CoroutineScope(Dispatchers.IO)
    private var loop: Job? = null
    private lateinit var capture: SensorCapture

    override fun onCreate() {
        super.onCreate()
        capture = SensorCapture(applicationContext)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForegroundCompat(buildNotification("Starting sensor node…"))
        capture.start()
        if (loop?.isActive != true) loop = scope.launch { runLoop() }
        return START_STICKY
    }

    private suspend fun runLoop() {
        while (scope.isActive) {
            val avail = capture.availableCount()
            updateNotification("$avail sensors · read-only capture")
            delay(COMMIT_INTERVAL_MS)
        }
    }

    override fun onDestroy() {
        capture.stop()
        loop?.cancel()
        scope.cancel()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun startForegroundCompat(n: Notification) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(NOTIF_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
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
        private const val CHANNEL_ID = "hone_node"
        private const val NOTIF_ID = 1001
        private const val COMMIT_INTERVAL_MS = 30_000L

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

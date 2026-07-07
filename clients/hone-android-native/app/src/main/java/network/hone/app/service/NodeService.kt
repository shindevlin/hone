package network.hone.app.service

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat

/**
 * Foreground service that KEEPS THE HONE NODE ALIVE when the app is backgrounded.
 *
 * This is the piece the old webview fundamentally could not do: Android kills a
 * webview when it loses focus, so the phone stopped mining/clocking the moment
 * the user switched apps. A foreground service with an ongoing notification lets
 * the phone be the full node HONE specifies (miner + clock + sensor).
 *
 * Phase 1 status: SKELETON. It runs as a foreground service with a live
 * notification. Phase 0b wires the actual Rust node (`HoneNode.start()` from the
 * uniffi bridge) into onStartCommand, and pushes live epoch/balance into the
 * notification. WorkManager (Phase 1) will restart this on boot / after kill.
 */
class NodeService : Service() {

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        startForeground(NOTIF_ID, buildNotification("Starting…"))
        // TODO(phase-0b): honeNode = HoneNode(); honeNode.start(configFromPrefs())
        //                 then observe status and updateNotification() on each epoch.
        return START_STICKY // Android restarts the service if it's killed.
    }

    override fun onDestroy() {
        // TODO(phase-0b): honeNode.stop()
        super.onDestroy()
    }

    private fun buildNotification(status: String): Notification =
        NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("HONE node")
            .setContentText(status)
            .setSmallIcon(android.R.drawable.ic_lock_idle_charging) // placeholder icon
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        private const val CHANNEL_ID = "hone_node"
        private const val NOTIF_ID = 1001

        /** Create the notification channel (call once, e.g. from Application). */
        fun ensureChannel(ctx: Context) {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                val channel = NotificationChannel(
                    CHANNEL_ID, "HONE node",
                    NotificationManager.IMPORTANCE_LOW,
                ).apply { description = "Keeps your node mining/clocking in the background." }
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

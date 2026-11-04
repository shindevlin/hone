package network.hone.app.station

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.net.wifi.WifiManager
import android.os.BatteryManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import network.hone.app.R
import network.hone.app.telemetry.Telemetry

/**
 * Foreground service for camera bird stations. Listens only while the phone is charging and
 * on Wi-Fi; otherwise every camera connection is closed. Audio stays in memory until
 * classified; nothing is recorded or sent except species names and times.
 */
class StationService : Service() {
    private val listeners = HashMap<String, StationListener>()
    @Volatile private var loop: Thread? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private var wake: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        ensureChannel(this)
        val n = notif(getString(R.string.nt_station_service))
        if (Build.VERSION.SDK_INT >= 34) runCatching { startForeground(NOTIF, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE) }
        else startForeground(NOTIF, n)
        Telemetry.installCrashHandler(applicationContext)
        if (loop?.isAlive != true) loop = Thread({ run() }, "hone-stations").apply { isDaemon = true; start() }
        return START_STICKY
    }

    private fun run() {
        while (!Thread.currentThread().isInterrupted) {
            val stations = StationStore.list(this).filter { it.enabled }
            if (stations.isEmpty()) { stopAll(); stopSelf(); return }
            val ok = conditionsMet(this)
            synchronized(listeners) {
                if (ok) {
                    hold(true)
                    for (s in stations) listeners.getOrPut(s.id) { StationListener(applicationContext, s) }.start()
                    (listeners.keys - stations.map { it.id }.toSet()).forEach { listeners.remove(it)?.stop() }
                } else { listeners.values.forEach { it.stop() }; hold(false) }
            }
            val live = Stations.live.value
            val text = if (!ok) getString(R.string.st_paused) else {
                val birds = stations.sumOf { live[it.id]?.birds ?: 0 }
                val dropped = stations.sumOf { live[it.id]?.speechDropped ?: 0 }
                getString(R.string.st_stats, birds.toInt(), dropped.toInt())
            }
            getSystemService(NotificationManager::class.java).notify(NOTIF, notif(text))
            runCatching { StationSync.maybeFlush(applicationContext) }.onFailure { Telemetry.error(applicationContext, "station sync", it) }
            runCatching { network.hone.app.telemetry.DeviceStatus.maybeSend(applicationContext) }
            try { Thread.sleep(30_000) } catch (_: InterruptedException) { break }
        }
    }

    private fun hold(on: Boolean) {
        if (on) {
            if (wifiLock == null) wifiLock = getSystemService(WifiManager::class.java)?.createWifiLock(
                if (Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_LOW_LATENCY else @Suppress("DEPRECATION") WifiManager.WIFI_MODE_FULL_HIGH_PERF,
                "hone:station")?.apply { setReferenceCounted(false); acquire() }
            if (wake == null) wake = getSystemService(PowerManager::class.java)?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "hone:station")
                ?.apply { setReferenceCounted(false); acquire() }
        } else {
            runCatching { wifiLock?.release() }; wifiLock = null
            runCatching { wake?.release() }; wake = null
        }
    }

    private fun stopAll() { synchronized(listeners) { listeners.values.forEach { it.stop() }; listeners.clear() }; StationAnalyzer.stop(); hold(false) }

    override fun onDestroy() { loop?.interrupt(); stopAll(); super.onDestroy() }

    private fun notif(text: String): Notification = NotificationCompat.Builder(this, CHANNEL)
        .setContentTitle(getString(R.string.nt_station_service)).setContentText(text)
        .setSmallIcon(android.R.drawable.ic_btn_speak_now).setOngoing(true).setPriority(NotificationCompat.PRIORITY_LOW).build()

    companion object {
        private const val CHANNEL = "hone_station"
        private const val NOTIF = 1002

        fun ensureChannel(ctx: Context) {
            if (Build.VERSION.SDK_INT >= 26) ctx.getSystemService(NotificationManager::class.java).createNotificationChannel(
                NotificationChannel(CHANNEL, ctx.getString(R.string.nt_station_service), NotificationManager.IMPORTANCE_LOW))
        }

        fun charging(ctx: Context): Boolean =
            (ctx.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))?.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) ?: 0) != 0

        fun onWifi(ctx: Context): Boolean {
            val cm = ctx.getSystemService(ConnectivityManager::class.java) ?: return false
            return cm.allNetworks.any { cm.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true }
        }

        fun conditionsMet(ctx: Context) = charging(ctx) && onWifi(ctx)

        fun start(ctx: Context) {
            if (StationStore.list(ctx).none { it.enabled }) return
            val i = Intent(ctx, StationService::class.java)
            runCatching { if (Build.VERSION.SDK_INT >= 26) ctx.startForegroundService(i) else ctx.startService(i) }
        }

        fun stop(ctx: Context) { ctx.stopService(Intent(ctx, StationService::class.java)) }
    }
}

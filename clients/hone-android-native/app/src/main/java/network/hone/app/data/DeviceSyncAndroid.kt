package network.hone.app.data

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import androidx.core.app.NotificationCompat
import network.hone.app.MainActivity
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * Android side of sync before submit: the device-log store (files/hone-node/devlog), the engines
 * (public node list, or the sensor node the owner configured), and the owner notification.
 */
object DeviceSync {
    private const val TAG = "HoneDeviceSync"
    private const val CHANNEL = "hone_device_history"
    const val MISMATCH_TEXT =
        "This device's history doesn't match the chain — it may have been restored or copied. Submissions paused."

    @Volatile private var store: FileDeviceLogStore? = null
    @Volatile private var public: DeviceSyncEngine? = null
    private val byNode = HashMap<String, DeviceSyncEngine>()

    fun store(ctx: Context): FileDeviceLogStore = store ?: synchronized(this) {
        store ?: FileDeviceLogStore(File(ctx.applicationContext.filesDir, "hone-node/devlog")).also { store = it }
    }

    /** Engine for commits sent through ChainHttp (public node first). */
    fun engine(ctx: Context): DeviceSyncEngine = public ?: synchronized(this) {
        public ?: DeviceSyncEngine(store(ctx), HttpHeadSource { ChainHttp.getWithCode(it) }, alerter(ctx)).also { public = it }
    }

    /** Engine for commits sent to one configured node (sensor settings node_url). */
    fun engine(ctx: Context, nodeUrl: String): DeviceSyncEngine = synchronized(this) {
        val base = nodeUrl.trimEnd('/')
        byNode.getOrPut(base) { DeviceSyncEngine(store(ctx), HttpHeadSource { getWithCode(base + it) }, alerter(ctx)) }
    }

    /** Keys whose submissions are paused (history mismatch), with the reason. */
    fun paused(ctx: Context): Map<String, String> =
        store(ctx).all().filterValues { it.paused }.mapValues { it.value.reason }

    /** Owner's re-anchor for one key. False when no node answered. */
    fun reanchor(ctx: Context, key: String): Boolean {
        val ok = engine(ctx).reanchor(key)
        if (ok) synchronized(this) { byNode.values.forEach { it.invalidate(key) } }
        if (ok) cancel(ctx, key)
        return ok
    }

    private fun alerter(ctx: Context): (String, String) -> Unit {
        val app = ctx.applicationContext
        return { key, reason ->
            Log.w(TAG, "history mismatch for $key: $reason")
            runCatching { network.hone.app.telemetry.Telemetry.skipped(app, "device_history", "$key: $reason") }
            notifyMismatch(app, key)
        }
    }

    fun notifyMismatch(ctx: Context, key: String) {
        if (Build.VERSION.SDK_INT >= 33 && ctx.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
        if (Build.VERSION.SDK_INT >= 26) nm.createNotificationChannel(
            NotificationChannel(CHANNEL, "Device history", NotificationManager.IMPORTANCE_HIGH))
        val pi = PendingIntent.getActivity(ctx, key.hashCode(), Intent(ctx, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE)
        nm.notify(key, NOTIFY_ID, NotificationCompat.Builder(ctx, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_warning)
            .setContentTitle("HONE: $key paused")
            .setContentText(MISMATCH_TEXT)
            .setStyle(NotificationCompat.BigTextStyle().bigText("$MISMATCH_TEXT\n\nSettings → Device history → Re-anchor, once you are sure this is the only copy."))
            .setContentIntent(pi).setAutoCancel(false).build())
    }

    fun cancel(ctx: Context, key: String) {
        ctx.getSystemService(NotificationManager::class.java)?.cancel(key, NOTIFY_ID)
    }

    private const val NOTIFY_ID = 7311

    private fun getWithCode(url: String): Pair<Int, String?>? = runCatching {
        val c = URL(url).openConnection() as HttpURLConnection
        c.connectTimeout = 5_000; c.readTimeout = 10_000
        try {
            val code = c.responseCode
            code to (if (code in 200..299) c.inputStream.bufferedReader().use { it.readText() } else null)
        } finally { c.disconnect() }
    }.getOrNull()
}

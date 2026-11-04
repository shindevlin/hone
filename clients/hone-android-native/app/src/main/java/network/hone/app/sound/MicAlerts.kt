package network.hone.app.sound

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import network.hone.app.MainActivity
import network.hone.app.R

/** Tells the user when listening stops for a reason only they can fix. */
object MicAlerts {
    private const val CHANNEL = "hone_alerts"
    private const val ID_SILENCED = 2101
    private const val ID_BATTERY = 2102
    @Volatile private var silencedShown = false
    @Volatile private var batteryShown = 0   // 0 none, 1 low (paused), 2 low while charging

    fun silenced(ctx: Context) {
        if (silencedShown) return
        silencedShown = true
        post(ctx, ID_SILENCED, ctx.getString(R.string.nt_mic_paused), ctx.getString(R.string.nt_mic_paused_body), true)
    }

    fun clearSilenced(ctx: Context) {
        if (!silencedShown) return
        silencedShown = false
        ctx.getSystemService(NotificationManager::class.java).cancel(ID_SILENCED)
    }

    /** Below 15%: warn. Unplugged, listening pauses; plugged but still low, the charger is too weak. */
    fun battery(ctx: Context, pct: Int?, plugged: Boolean) {
        val want = when {
            pct == null || pct >= SoundSensing.LOW_BATTERY_PCT + 5 -> 0
            pct >= SoundSensing.LOW_BATTERY_PCT -> batteryShown   // hysteresis
            plugged -> 2
            else -> 1
        }
        if (want == batteryShown) return
        batteryShown = want
        if (want == 0) { ctx.getSystemService(NotificationManager::class.java).cancel(ID_BATTERY); return }
        val text = if (want == 2) ctx.getString(R.string.nt_battery_weak, pct) else ctx.getString(R.string.nt_battery_low, pct)
        post(ctx, ID_BATTERY, ctx.getString(R.string.nt_battery_title), text, false)
    }

    private fun post(ctx: Context, id: Int, title: String, text: String, ongoing: Boolean) {
        if (Build.VERSION.SDK_INT >= 33 && ctx.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val nm = ctx.getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) nm.createNotificationChannel(
            NotificationChannel(CHANNEL, ctx.getString(R.string.nt_alerts_channel), NotificationManager.IMPORTANCE_DEFAULT))
        // Opening the app brings the node service back to the foreground with the microphone type.
        val pi = PendingIntent.getActivity(ctx, id, Intent(ctx, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP), PendingIntent.FLAG_IMMUTABLE)
        nm.notify(id, NotificationCompat.Builder(ctx, CHANNEL).setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentTitle(title).setContentText(text).setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setContentIntent(pi).setAutoCancel(true).setOngoing(ongoing).build())
    }
}

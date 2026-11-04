package network.hone.app.engage

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import network.hone.app.MainActivity
import network.hone.app.R
import network.hone.app.birds.Birds
import network.hone.app.birds.BirdsSync
import network.hone.app.account.AccountStore
import org.json.JSONObject
import java.io.File
import java.text.DateFormat
import java.util.Calendar
import java.util.Date
import java.util.concurrent.TimeUnit

/**
 * Life list and bird notifications: first bird, 5 and 10 species, first in your area, a
 * morning summary of last night (opt-out), and the rarest species of the month. A species
 * joins the life list at 70 %+ confidence (the same bar as public sightings).
 */
object Engagement {
    private const val CHANNEL = "hone_birds"
    const val LIFE_MIN = 0.70f
    private val MILESTONES = setOf(5, 10, 25, 50, 100)

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_engage", Context.MODE_PRIVATE)

    fun digestEnabled(ctx: Context) = prefs(ctx).getBoolean("digest", true)
    fun setDigest(ctx: Context, on: Boolean) = prefs(ctx).edit().putBoolean("digest", on).apply()

    @Synchronized
    fun lifeList(ctx: Context): List<Pair<String, Long>> {
        val j = runCatching { JSONObject(prefs(ctx).getString("life", "{}")!!) }.getOrDefault(JSONObject())
        return j.keys().asSequence().map { it to j.getLong(it) }.sortedBy { it.second }.toList()
    }

    fun displayName(ctx: Context, latin: String) = runCatching { Birds.english(ctx, latin) }.getOrNull() ?: latin

    /** Called for every detection from the phone mic or a station. */
    @Synchronized
    fun onDetection(ctx: Context, latin: String, tMs: Long, conf: Float = 1f) {
        if (conf < LIFE_MIN) return
        val p = prefs(ctx)
        val life = runCatching { JSONObject(p.getString("life", "{}")!!) }.getOrDefault(JSONObject())
        if (life.has(latin)) return
        life.put(latin, tMs)
        p.edit().putString("life", life.toString()).apply()
        val n = life.length()
        if (n == 1 && !p.getBoolean("first_done", false)) {
            p.edit().putBoolean("first_done", true).apply()
            notify(ctx, 2001, ctx.getString(R.string.nt_first_bird, displayName(ctx, latin),
                DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(tMs))))
        } else if (n in MILESTONES) notify(ctx, 2002, ctx.getString(R.string.nt_milestone, n))
    }

    /** The indexer said nobody had reported this species in the area before. */
    @Synchronized
    fun firstInArea(ctx: Context, latin: String) {
        val p = prefs(ctx); val k = "area_$latin"
        if (p.getBoolean(k, false)) return
        p.edit().putBoolean(k, true).apply()
        notify(ctx, 2003 + (latin.hashCode() and 0xff), ctx.getString(R.string.nt_first_area, displayName(ctx, latin)))
    }

    fun notify(ctx: Context, id: Int, text: String) {
        if (Build.VERSION.SDK_INT >= 33 && ctx.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) return
        val nm = ctx.getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= 26) nm.createNotificationChannel(NotificationChannel(CHANNEL, ctx.getString(R.string.nt_channel), NotificationManager.IMPORTANCE_DEFAULT))
        val pi = PendingIntent.getActivity(ctx, 0, Intent(ctx, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        nm.notify(id, NotificationCompat.Builder(ctx, CHANNEL).setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentTitle("HONE").setContentText(text).setStyle(NotificationCompat.BigTextStyle().bigText(text))
            .setContentIntent(pi).setAutoCancel(true).build())
    }

    /** All detection logs on this phone: (time, latin, conf). */
    fun detections(ctx: Context, sinceMs: Long): List<Triple<Long, String, Float>> {
        val files = listOf(File(ctx.filesDir, "hone-node/sound/detections.jsonl")) +
            File(ctx.filesDir, "stations").listFiles().orEmpty().filter { it.isDirectory }.map { File(it, "detections.jsonl") }
        val out = ArrayList<Triple<Long, String, Float>>()
        for (f in files) if (f.exists()) f.forEachLine { l ->
            val j = runCatching { JSONObject(l) }.getOrNull() ?: return@forEachLine
            val t = j.optLong("t"); if (t >= sinceMs) out += Triple(t, j.optString("name"), (j.optDouble("conf", 0.0) / 100).toFloat())
        }
        return out
    }

    fun scheduleDigest(ctx: Context) {
        val now = Calendar.getInstance()
        val next = (now.clone() as Calendar).apply { set(Calendar.HOUR_OF_DAY, 7); set(Calendar.MINUTE, 30); set(Calendar.SECOND, 0)
            if (before(now)) add(Calendar.DAY_OF_YEAR, 1) }
        val req = PeriodicWorkRequestBuilder<DigestWorker>(1, TimeUnit.DAYS)
            .setInitialDelay(next.timeInMillis - now.timeInMillis, TimeUnit.MILLISECONDS).build()
        WorkManager.getInstance(ctx).enqueueUniquePeriodicWork("hone-digest", ExistingPeriodicWorkPolicy.KEEP, req)
    }
}

/** Morning summary of last night (18:00 to now), and the rarest species once a month. */
class DigestWorker(ctx: Context, params: WorkerParameters) : CoroutineWorker(ctx, params) {
    override suspend fun doWork(): Result {
        val c = applicationContext
        if (!Engagement.digestEnabled(c)) return Result.success()
        val start = Calendar.getInstance().apply { add(Calendar.DAY_OF_YEAR, -1); set(Calendar.HOUR_OF_DAY, 18); set(Calendar.MINUTE, 0) }.timeInMillis
        val night = Engagement.detections(c, start).filter { it.third >= Engagement.LIFE_MIN }
        val species = night.map { it.second }.toSet()
        val newOnes = Engagement.lifeList(c).count { it.second >= start }
        if (species.isNotEmpty()) Engagement.notify(c, 2100, c.getString(R.string.nt_digest, species.size, night.size, newOnes))
        // Once a month: the rarest species heard here this month (fewest public sightings worldwide).
        val p = c.getSharedPreferences("hone_engage", Context.MODE_PRIVATE)
        val month = Calendar.getInstance().let { it.get(Calendar.YEAR) * 100 + it.get(Calendar.MONTH) }
        if (Calendar.getInstance().get(Calendar.DAY_OF_MONTH) >= 28 && p.getInt("rarest_month", 0) != month) runCatching {
            val m0 = Calendar.getInstance().apply { set(Calendar.DAY_OF_MONTH, 1); set(Calendar.HOUR_OF_DAY, 0) }.timeInMillis
            val mine = Engagement.detections(c, m0).filter { it.third >= Engagement.LIFE_MIN }.map { it.second }.toSet()
            val counts = org.json.JSONArray(BirdsSync.getJson("/species")).let { a ->
                (0 until a.length()).associate { a.getJSONObject(it).getString("latin") to a.getJSONObject(it).getInt("sightings") }
            }
            mine.minByOrNull { counts[it] ?: 0 }?.let { Engagement.notify(c, 2101, c.getString(R.string.nt_rarest, Engagement.displayName(c, it))) }
            p.edit().putInt("rarest_month", month).apply()
        }
        AccountStore.syncProfile(c)
        return Result.success()
    }
}

package network.hone.app.update

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInfo
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.core.content.pm.PackageInfoCompat
import androidx.work.*
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import network.hone.app.MainActivity
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.TimeUnit

/**
 * In-app updates from https://honemesh.net/app/version.json ({versionCode, sha256, url}).
 * An update is offered only when versionCode is newer than the installed one. The APK is
 * downloaded over HTTPS, its SHA-256 must equal the published value AND its signing
 * certificate must match the installed app's before it is handed to the system
 * installer, where the user still taps Install.
 */
object AppUpdate {
    const val VERSION_URL = "https://honemesh.net/app/version.json"
    private const val CHANNEL = "hone_update"
    private const val WORK = "hone-update-check"

    data class Info(val versionCode: Long, val sha256: String, val url: String)

    fun installedVersion(ctx: Context): Long =
        PackageInfoCompat.getLongVersionCode(ctx.packageManager.getPackageInfo(ctx.packageName, 0))

    private fun fetch(): Info? = runCatching {
        val c = URL(VERSION_URL).openConnection() as HttpURLConnection
        c.connectTimeout = 8_000; c.readTimeout = 10_000
        c.setRequestProperty("Cache-Control", "no-cache")
        try {
            if (c.responseCode !in 200..299) null
            else JSONObject(c.inputStream.bufferedReader().use { it.readText() }).let {
                Info(it.getLong("versionCode"), it.getString("sha256").lowercase(), it.getString("url"))
            }
        } finally { c.disconnect() }
    }.getOrNull()

    /** The published update if it is newer than this install (blocking; call off the main thread). */
    fun available(ctx: Context): Info? = fetch()?.takeIf {
        it.versionCode > installedVersion(ctx) && it.url.startsWith("https://") &&
            it.sha256.matches(Regex("[0-9a-f]{64}"))
    }

    private fun hex(b: ByteArray) = b.joinToString("") { "%02x".format(it) }

    @Suppress("DEPRECATION")
    private fun certDigests(pi: PackageInfo?): Set<String> {
        if (pi == null) return emptySet()
        val sigs = if (Build.VERSION.SDK_INT >= 28) pi.signingInfo?.let {
            if (it.hasMultipleSigners()) it.apkContentsSigners else it.signingCertificateHistory
        } else pi.signatures
        val md = MessageDigest.getInstance("SHA-256")
        return sigs.orEmpty().map { hex(md.digest(it.toByteArray())) }.toSet()
    }

    /** The downloaded APK is this app (same package) signed with the installed app's certificate. */
    @Suppress("DEPRECATION")
    fun sameSigner(ctx: Context, apk: File): Boolean {
        val pm = ctx.packageManager
        val flags = if (Build.VERSION.SDK_INT >= 28) PackageManager.GET_SIGNING_CERTIFICATES else PackageManager.GET_SIGNATURES
        val installed = certDigests(pm.getPackageInfo(ctx.packageName, flags))
        val archive = pm.getPackageArchiveInfo(apk.path, flags) ?: return false
        if (archive.packageName != ctx.packageName) return false
        var a = certDigests(archive)
        if (a.isEmpty()) a = certDigests(pm.getPackageArchiveInfo(apk.path, PackageManager.GET_SIGNATURES))
        return installed.isNotEmpty() && a.isNotEmpty() && a.any { it in installed }
    }

    /** Download and verify; returns the verified APK or throws. Blocking. */
    fun download(ctx: Context, info: Info, progress: (Long, Long) -> Unit = { _, _ -> }): File {
        val dir = File(ctx.cacheDir, "updates").apply { mkdirs() }
        dir.listFiles()?.forEach { it.delete() }
        val out = File(dir, "hone-${info.versionCode}.apk")
        val md = MessageDigest.getInstance("SHA-256")
        val c = URL(info.url).openConnection() as HttpURLConnection
        c.connectTimeout = 10_000; c.readTimeout = 30_000
        try {
            check(c.responseCode in 200..299) { "download failed (HTTP ${c.responseCode})" }
            val total = c.contentLengthLong
            c.inputStream.use { ins ->
                out.outputStream().use { os ->
                    val buf = ByteArray(64 * 1024); var got = 0L
                    while (true) {
                        val n = ins.read(buf); if (n < 0) break
                        os.write(buf, 0, n); md.update(buf, 0, n); got += n; progress(got, total)
                    }
                }
            }
        } finally { c.disconnect() }
        if (hex(md.digest()) != info.sha256) { out.delete(); error("checksum mismatch") }
        if (!sameSigner(ctx, out)) { out.delete(); error("signing certificate does not match this app") }
        return out
    }

    fun canInstall(ctx: Context) = ctx.packageManager.canRequestPackageInstalls()

    fun openInstallPermission(ctx: Context) = ctx.startActivity(
        Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:" + ctx.packageName))
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))

    /** Hand the verified APK to the system installer; the user confirms there. */
    fun install(ctx: Context, apk: File) {
        val uri = FileProvider.getUriForFile(ctx, ctx.packageName + ".updates", apk)
        ctx.startActivity(Intent(Intent.ACTION_VIEW).setDataAndType(uri, "application/vnd.android.package-archive")
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK))
    }

    /** Daily background check that posts a notification when an update is out. */
    fun schedule(ctx: Context) {
        val req = PeriodicWorkRequestBuilder<UpdateWorker>(1, TimeUnit.DAYS)
            .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
            .build()
        WorkManager.getInstance(ctx).enqueueUniquePeriodicWork(WORK, ExistingPeriodicWorkPolicy.KEEP, req)
    }

    internal fun notify(ctx: Context) {
        if (Build.VERSION.SDK_INT >= 33 && ContextCompat.checkSelfPermission(ctx, Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED) return
        val nm = ctx.getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "App updates", NotificationManager.IMPORTANCE_DEFAULT))
        val pi = PendingIntent.getActivity(ctx, 7, Intent(ctx, MainActivity::class.java)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP), PendingIntent.FLAG_IMMUTABLE)
        nm.notify(7101, NotificationCompat.Builder(ctx, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download_done)
            .setContentTitle("HONE update available")
            .setContentText("Open HONE to download and install it.")
            .setContentIntent(pi).setAutoCancel(true).build())
    }
}

class UpdateWorker(ctx: Context, params: WorkerParameters) : CoroutineWorker(ctx, params) {
    override suspend fun doWork(): Result {
        if (withContext(Dispatchers.IO) { AppUpdate.available(applicationContext) } != null) AppUpdate.notify(applicationContext)
        return Result.success()
    }
}

/** "Update available" bar: checks on start and every 24 h while open. */
@Composable
fun UpdateBanner() {
    val ctx = LocalContext.current
    var info by remember { mutableStateOf<AppUpdate.Info?>(null) }
    var status by remember { mutableStateOf("") }
    var busy by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(Unit) {
        runCatching { AppUpdate.schedule(ctx) }
        while (true) {
            info = withContext(Dispatchers.IO) { AppUpdate.available(ctx) }
            delay(TimeUnit.DAYS.toMillis(1))
        }
    }
    val i = info ?: return
    Surface(color = MaterialTheme.colorScheme.primaryContainer, modifier = Modifier.fillMaxWidth().statusBarsPadding()) {
        Row(Modifier.padding(horizontal = 16.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(status.ifEmpty { "Update available" }, Modifier.weight(1f), style = MaterialTheme.typography.bodyMedium)
            TextButton(enabled = !busy, onClick = {
                if (!AppUpdate.canInstall(ctx)) { status = "Allow installs from HONE, then tap Install"; AppUpdate.openInstallPermission(ctx); return@TextButton }
                busy = true
                scope.launch {
                    status = "Downloading…"
                    val r = withContext(Dispatchers.IO) {
                        runCatching { AppUpdate.download(ctx, i) { got, total -> if (total > 0) status = "Downloading ${got * 100 / total}%" } }
                    }
                    busy = false
                    r.onSuccess { status = "Verified, opening installer"; AppUpdate.install(ctx, it) }
                        .onFailure { status = "Update failed: ${it.message}" }
                }
            }) { Text("Install") }
        }
    }
}
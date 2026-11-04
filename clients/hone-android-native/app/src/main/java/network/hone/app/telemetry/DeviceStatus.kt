package network.hone.app.telemetry

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.BatteryManager
import android.os.Build
import android.os.PowerManager
import android.os.Process
import android.os.SystemClock
import android.util.Log
import network.hone.app.BuildConfig
import network.hone.app.data.HoneAnnounceScanner
import network.hone.app.sightings.Cells
import network.hone.app.sightings.SightingsEngine
import network.hone.app.sound.SoundSensing
import org.json.JSONObject
import java.io.File

/**
 * Signed device status ("heartbeat"): every 15 min and on a state change (at most once a
 * minute), POST https://honemesh.net/api/devices/status. Contents are counts, ages, on/off
 * states and short error strings; never keys, audio, location trails or other people's ids.
 * Location is at most the H3 res-7 cell (~5 km) of the latest fix.
 */
object DeviceStatus {
    private const val TAG = "HoneStatus"
    const val URL = "${Telemetry.API}/devices/status"
    const val PERIOD_MS = 15 * 60_000L
    private const val MIN_GAP_MS = 60_000L

    @Volatile private var lastSentMs = 0L
    @Volatile private var lastState = ""

    /** Called every service loop (30 s) and by StatusWorker. Sends when due or changed. */
    @Synchronized
    fun maybeSend(ctx: Context, force: Boolean = false) {
        val now = System.currentTimeMillis()
        val st = runCatching { build(ctx, now) }.getOrElse { Telemetry.error(ctx, "status build", it); return }
        val state = Canonical.encode(st["state"])
        val due = force || now - lastSentMs >= PERIOD_MS
        val changed = state != lastState && now - lastSentMs >= MIN_GAP_MS
        if (!due && !changed) return
        val account = Telemetry.account(ctx) ?: Telemetry.seatAccount(ctx)
        if (account == null) { Telemetry.skipped(ctx, "status", "no account on this device"); return }
        val body = runCatching { Telemetry.signedBody(ctx, account, st + ("account" to account)) }
            .getOrElse { Telemetry.error(ctx, "status sign", it); return }
        val r = Telemetry.post(URL, body)
        when {
            r == null -> Telemetry.upload(ctx, "status", false, "offline")
            r.code in 200..299 -> { Telemetry.upload(ctx, "status", true); lastSentMs = now; lastState = state }
            else -> { Telemetry.upload(ctx, "status", false, "HTTP ${r.code} ${r.body.take(120)}"); lastSentMs = now; lastState = state }
        }
        Log.i(TAG, "status -> ${r?.code ?: "offline"}")
    }

    private fun granted(ctx: Context, p: String) = ctx.checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    private fun ageS(ms: Long, now: Long): Long? = if (ms <= 0) null else (now - ms) / 1000

    fun build(ctx: Context, now: Long): Map<String, Any?> {
        val pi = ctx.packageManager.getPackageInfo(ctx.packageName, 0)
        val vc = if (Build.VERSION.SDK_INT >= 28) pi.longVersionCode else @Suppress("DEPRECATION") pi.versionCode.toLong()

        val cm = ctx.getSystemService(ConnectivityManager::class.java)
        val caps = runCatching { cm?.getNetworkCapabilities(cm.activeNetwork) }.getOrNull()
        val netType = when {
            caps == null -> "none"
            caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) -> "wifi"
            caps.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR) -> "cellular"
            caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) -> "ethernet"
            caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN) -> "vpn"
            else -> "other"
        }

        val fix = SightingsEngine.lastFix()
        val fixAge = fix?.let { (SystemClock.elapsedRealtimeNanos() - it.elapsedRealtimeNanos) / 1_000_000_000 }
        val cell7 = fix?.takeIf { (fixAge ?: Long.MAX_VALUE) < 3600 }?.let { Cells.cell(it.latitude, it.longitude, 7) }

        val sound = SoundSensing.live.value
        val soundOn = SoundSensing.isEnabled(ctx)
        val mic = granted(ctx, Manifest.permission.RECORD_AUDIO)
        val perchRunning = soundOn && mic && (sound.listening || sound.status == "waiting" || sound.status == "listening")

        val bleScanPerm = if (Build.VERSION.SDK_INT >= 31) granted(ctx, Manifest.permission.BLUETOOTH_SCAN)
            else granted(ctx, Manifest.permission.ACCESS_FINE_LOCATION)
        val btOn = runCatching {
            ctx.getSystemService(android.bluetooth.BluetoothManager::class.java)?.adapter?.isEnabled == true
        }.getOrDefault(false)

        val devs = HoneAnnounceScanner.devices.value.values
        val flipLast = devs.maxOfOrNull { it.lastSeenMs } ?: 0L

        val bat = ctx.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val level = bat?.getIntExtra(BatteryManager.EXTRA_LEVEL, -1) ?: -1
        val scale = bat?.getIntExtra(BatteryManager.EXTRA_SCALE, 100) ?: 100
        val plugged = (bat?.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) ?: 0) != 0
        val pct = if (level >= 0 && scale > 0) level * 100 / scale else null
        val optExempt = ctx.getSystemService(PowerManager::class.java)?.isIgnoringBatteryOptimizations(ctx.packageName) == true

        val perms = linkedMapOf(
            "location_fine" to granted(ctx, Manifest.permission.ACCESS_FINE_LOCATION),
            "location_background" to (Build.VERSION.SDK_INT < 29 || granted(ctx, Manifest.permission.ACCESS_BACKGROUND_LOCATION)),
            "record_audio" to mic,
            "bluetooth_scan" to bleScanPerm,
            "bluetooth_advertise" to (Build.VERSION.SDK_INT < 31 || granted(ctx, Manifest.permission.BLUETOOTH_ADVERTISE)),
            "notifications" to (Build.VERSION.SDK_INT < 33 || granted(ctx, Manifest.permission.POST_NOTIFICATIONS)),
            "camera" to granted(ctx, Manifest.permission.CAMERA),
        )

        val account = Telemetry.account(ctx)
        val posting = Telemetry.postingState(ctx, account)
        val (summaries, queued) = runCatching { SightingsEngine.storeOrNull()?.summaryCounts() }.getOrNull() ?: (0 to 0)
        val birds = birdsCounts(ctx, now)
        val nodeUrl = runCatching {
            JSONObject(File(ctx.filesDir, "hone-node/sensor_settings.json").readText()).optString("node_url", "")
        }.getOrDefault("")

        // Sensors on/off + network + power + permissions: a change here sends a status early.
        val state = linkedMapOf<String, Any?>(
            "sound" to soundOn, "mic" to mic, "perch" to perchRunning, "gps" to (fix != null && (fixAge ?: 9999) < 600),
            "ble" to (btOn && bleScanPerm), "flipper" to (flipLast > 0 && now - flipLast < 300_000),
            "net" to netType, "charging" to plugged, "opt_exempt" to optExempt, "posting" to posting, "perms" to perms,
            "service" to (Telemetry.serviceStartedMs > 0),
        )

        return linkedMapOf(
            "v" to 1, "kind" to "device_status", "ts" to now / 1000,
            "device" to Telemetry.deviceId(ctx), "seat_pub" to Telemetry.seatPub(ctx), "seat_account" to Telemetry.seatAccount(ctx),
            "label" to Build.MODEL,
            "app" to mapOf("version_code" to vc, "version_name" to pi.versionName, "commit" to BuildConfig.GIT_COMMIT,
                "package" to ctx.packageName),
            "uptime_s" to (SystemClock.elapsedRealtime() - Process.getStartElapsedRealtime()) / 1000,
            "service_uptime_s" to ageS(Telemetry.serviceStartedMs, now),
            "android" to Build.VERSION.SDK_INT,
            "net" to mapOf("type" to netType,
                "validated" to (caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED) == true),
                "metered" to (caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) == false)),
            "cell7" to cell7,
            "sensors" to mapOf(
                "sound" to mapOf("enabled" to soundOn, "mic_permission" to mic, "listening" to sound.listening,
                    "status" to sound.status.take(80), "last_event_s" to ageS(Telemetry.birdLastMs, now),
                    "detections_session" to Telemetry.birdDetections.get()),
                "perch" to mapOf("running" to perchRunning),
                "gps" to mapOf("fix_age_s" to fixAge, "acc_m" to fix?.accuracy?.toInt(), "permission" to perms["location_fine"]),
                "ble" to mapOf("on" to btOn, "permission" to bleScanPerm, "adverts" to SightingsEngine.crowdResults,
                    "last_event_s" to ageS(Telemetry.bleLastMs, now)),
                "wifi" to mapOf("scans" to Telemetry.wifiScans.get(), "last_event_s" to ageS(Telemetry.wifiLastMs, now)),
                "flipper" to mapOf("devices" to devs.size, "last_seen_s" to ageS(flipLast, now),
                    "frames" to Telemetry.flipperFrames.get(), "last_frame_s" to ageS(Telemetry.flipperLastMs, now),
                    "commits_relayed" to devs.sumOf { it.relayed }, "commits_pending_chain" to devs.sumOf { it.queued },
                    "index_pending" to FlipperPublisher.pending(ctx),
                    "index" to Telemetry.uploadStats(ctx, "flipper", now)),
            ),
            "birds" to mapOf("detections_today" to birds.first, "windows_pending" to birds.second,
                "photos_queued" to birds.third, "sightings" to Telemetry.uploadStats(ctx, "birds", now),
                "photos" to Telemetry.uploadStats(ctx, "photos", now)),
            "coverage" to mapOf("summaries" to summaries, "queued" to queued),
            "chain" to mapOf("sensor_accepted" to Telemetry.sensorAccepted.get(), "sensor_rejected" to Telemetry.sensorRejected.get(),
                "sensor_last_error" to Telemetry.sensorLastError.take(120).ifEmpty { null },
                "node_url_public" to nodeUrl.startsWith("https://")),
            "posting_key" to posting,
            "battery" to mapOf("pct" to pct, "charging" to plugged, "optimization_exempt" to optExempt),
            "permissions" to perms,
            "last_crash" to Telemetry.lastCrash(ctx, now),
            "last_error" to Telemetry.lastError(ctx, now),
            "status_uploads" to Telemetry.uploadStats(ctx, "status", now),
            "state" to state,
        )
    }

    /** (detections today UTC, quarter-hour windows not yet published, photos queued). */
    private fun birdsCounts(ctx: Context, now: Long): Triple<Int, Long, Int> {
        val f = File(ctx.filesDir, "hone-node/sound/detections.jsonl")
        val day0 = now - now % 86_400_000L
        var today = 0
        if (f.exists() && f.length() < 8_000_000) runCatching {
            f.forEachLine { l ->
                val i = l.indexOf("\"t\":")
                if (i >= 0) l.substring(i + 4).takeWhile { it.isDigit() }.toLongOrNull()?.let { if (it >= day0) today++ }
            }
        }
        val sent = ctx.getSharedPreferences("hone_birds", Context.MODE_PRIVATE).getLong("sent_window", -1)
        val pending = if (sent < 0) 0 else maxOf(0, now / 900_000L - 1 - sent)
        val photos = File(ctx.filesDir, "birds/photos").listFiles().orEmpty().sumOf { d ->
            d.listFiles { x -> x.extension == "json" }.orEmpty().count { runCatching { JSONObject(it.readText()).optString("status") == "queued" }.getOrDefault(false) }
        }
        return Triple(today, pending, photos)
    }
}

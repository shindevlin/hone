package network.hone.app.telemetry

import network.hone.app.raw.RawUploader
import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import network.hone.app.birds.BirdsSync
import network.hone.app.home.HomeState
import network.hone.app.station.StationService
import network.hone.app.station.StationSync
import java.util.concurrent.TimeUnit

/**
 * Every 15 min with any network: status heartbeat, then queued bird sightings/photos and
 * Flipper frames. Runs even when the sensor service was killed, so a dead service still
 * shows up (service_uptime_s null) instead of the device going silent.
 */
class StatusWorker(ctx: Context, params: WorkerParameters) : CoroutineWorker(ctx, params) {
    override suspend fun doWork(): Result {
        val c = applicationContext
        runCatching { HomeState.tick(c) }.onFailure { Telemetry.error(c, "worker home", it) }
        runCatching { DeviceStatus.maybeSend(c) }.onFailure { Telemetry.error(c, "worker status", it) }
        runCatching { BirdsSync.syncNow(c) }.onFailure { Telemetry.error(c, "worker birds", it) }
        runCatching { StationSync.maybeFlush(c) }.onFailure { Telemetry.error(c, "worker stations", it) }
        runCatching { StationService.start(c) }
        runCatching { RawUploader.maybeFlush(c) }.onFailure { Telemetry.error(c, "worker raw", it) }
        runCatching { CoverageSubmitter.maybeFlush(c) }.onFailure { Telemetry.error(c, "worker coverage", it) }
        runCatching { FlipperPublisher.maybeFlush(c) }.onFailure { Telemetry.error(c, "worker flipper", it) }
        return Result.success()
    }

    companion object {
        private const val WORK = "hone-status"

        fun schedule(ctx: Context) {
            val req = PeriodicWorkRequestBuilder<StatusWorker>(15, TimeUnit.MINUTES)
                .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                .build()
            WorkManager.getInstance(ctx).enqueueUniquePeriodicWork(WORK, ExistingPeriodicWorkPolicy.KEEP, req)
        }
    }
}

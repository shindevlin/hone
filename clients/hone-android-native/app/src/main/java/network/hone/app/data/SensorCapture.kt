package network.hone.app.data

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.location.Location
import android.location.LocationListener
import android.location.LocationManager
import androidx.core.content.ContextCompat
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlin.math.sqrt

/** One capturable channel on this phone, with its latest reading. */
data class SensorChannel(
    val id: String,              // stable sensor_id suffix, e.g. "gnss", "baro", "accel"
    val label: String,           // human label
    val honeType: String,        // sensor_type reported to the chain
    val unit: String,
    val available: Boolean,
    val value: Double = Double.NaN,   // representative numeric value for cross-validation
    val detail: String = "",          // human-readable live detail
    val samples: Long = 0,            // readings observed this session (→ reading_count)
)

/**
 * Reads THIS phone's real sensors. Enumerates what the hardware actually reports (via
 * SensorManager) plus fused GNSS location, and exposes the latest value per channel. No
 * mock values — a channel is `available=false` if the device lacks that sensor.
 *
 * Representative value convention (what the chain's 2-of-N cross-validation compares):
 *   gnss → latitude, motion → acceleration magnitude, baro → hPa, light → lux, etc.
 * The full reading detail is kept locally and hashed into batch_hash; only the
 * representative value and the count go on-chain.
 */
class SensorCapture(private val ctx: Context) : SensorEventListener, LocationListener {

    private val sm = ctx.getSystemService(Context.SENSOR_SERVICE) as SensorManager
    private val lm = ctx.getSystemService(Context.LOCATION_SERVICE) as LocationManager

    // Declared BEFORE _channels: discover() reads `onboard`, and Kotlin initializes
    // properties top-to-bottom, so _channels = MutableStateFlow(discover()) must come
    // after onboard or discover() sees a null list (runtime NPE, not a compile error).
    /** Which Android Sensor type backs each onboard channel we care about. */
    private val onboard = listOf(
        Triple("accel", Sensor.TYPE_ACCELEROMETER, "motion"),
        Triple("gyro", Sensor.TYPE_GYROSCOPE, "motion"),
        Triple("mag", Sensor.TYPE_MAGNETIC_FIELD, "magnetic"),
        Triple("baro", Sensor.TYPE_PRESSURE, "pressure"),
        Triple("light", Sensor.TYPE_LIGHT, "illuminance"),
        Triple("proximity", Sensor.TYPE_PROXIMITY, "proximity"),
        Triple("steps", Sensor.TYPE_STEP_COUNTER, "count"),
    )

    private val counts = HashMap<String, Long>()

    private val _channels = MutableStateFlow(discover())
    val channels: StateFlow<List<SensorChannel>> = _channels

    private fun labelFor(id: String) = when (id) {
        "gnss" -> "GNSS"; "accel" -> "Accelerometer"; "gyro" -> "Gyroscope"
        "mag" -> "Magnetometer"; "baro" -> "Barometer"; "light" -> "Light"
        "proximity" -> "Proximity"; "steps" -> "Step counter"; else -> id
    }
    private fun unitFor(id: String) = when (id) {
        "gnss" -> "° lat/lon"; "accel", "gyro" -> "m/s²"; "mag" -> "µT"
        "baro" -> "hPa"; "light" -> "lux"; "proximity" -> "cm"; "steps" -> "steps"; else -> ""
    }

    private fun discover(): List<SensorChannel> {
        val list = mutableListOf<SensorChannel>()
        // GNSS via location providers.
        val hasGnss = lm.allProviders.contains(LocationManager.GPS_PROVIDER)
        list += SensorChannel("gnss", labelFor("gnss"), "continuous", unitFor("gnss"), hasGnss)
        // Onboard sensors that actually exist on this device.
        for ((id, type, hone) in onboard) {
            val present = sm.getDefaultSensor(type) != null
            list += SensorChannel(id, labelFor(id), hone, unitFor(id), present)
        }
        return list
    }

    fun availableCount(): Int = _channels.value.count { it.available }

    fun start() {
        for ((_, type, _) in onboard) {
            sm.getDefaultSensor(type)?.let { sm.registerListener(this, it, SensorManager.SENSOR_DELAY_NORMAL) }
        }
        if (hasLocationPermission()) {
            try {
                lm.requestLocationUpdates(LocationManager.GPS_PROVIDER, 10_000L, 5f, this)
            } catch (_: SecurityException) { /* permission race; UI re-requests */ }
        }
    }

    fun stop() {
        sm.unregisterListener(this)
        try { lm.removeUpdates(this) } catch (_: SecurityException) {}
    }

    fun hasLocationPermission(): Boolean =
        ContextCompat.checkSelfPermission(ctx, Manifest.permission.ACCESS_FINE_LOCATION) ==
            PackageManager.PERMISSION_GRANTED

    override fun onSensorChanged(e: SensorEvent) {
        val (id, value, detail) = when (e.sensor.type) {
            Sensor.TYPE_ACCELEROMETER -> Triple("accel", magnitude(e.values), fmt3(e.values))
            Sensor.TYPE_GYROSCOPE -> Triple("gyro", magnitude(e.values), fmt3(e.values))
            Sensor.TYPE_MAGNETIC_FIELD -> Triple("mag", magnitude(e.values), fmt3(e.values))
            Sensor.TYPE_PRESSURE -> Triple("baro", e.values[0].toDouble(), "${round1(e.values[0])} hPa")
            Sensor.TYPE_LIGHT -> Triple("light", e.values[0].toDouble(), "${round1(e.values[0])} lux")
            Sensor.TYPE_PROXIMITY -> Triple("proximity", e.values[0].toDouble(), "${round1(e.values[0])} cm")
            Sensor.TYPE_STEP_COUNTER -> Triple("steps", e.values[0].toDouble(), "${e.values[0].toLong()} steps")
            else -> return
        }
        update(id, value, detail)
    }

    override fun onLocationChanged(loc: Location) {
        update("gnss", loc.latitude,
            "%.5f, %.5f  ±%.0fm".format(loc.latitude, loc.longitude, loc.accuracy))
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}
    @Deprecated("legacy") override fun onStatusChanged(p: String?, s: Int, e: android.os.Bundle?) {}
    override fun onProviderEnabled(provider: String) {}
    override fun onProviderDisabled(provider: String) {}

    private fun update(id: String, value: Double, detail: String) {
        val n = (counts[id] ?: 0L) + 1L
        counts[id] = n
        _channels.value = _channels.value.map {
            if (it.id == id) it.copy(value = value, detail = detail, samples = n) else it
        }
    }

    /** Snapshot + reset the sample count for a channel (called when a batch is committed). */
    fun drainSampleCount(id: String): Long {
        val n = counts[id] ?: 0L
        counts[id] = 0L
        return n
    }

    private fun magnitude(v: FloatArray) =
        sqrt((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).toDouble())
    private fun fmt3(v: FloatArray) = "x %.2f · y %.2f · z %.2f".format(v[0], v[1], v[2])
    private fun round1(f: Float) = "%.1f".format(f)
}

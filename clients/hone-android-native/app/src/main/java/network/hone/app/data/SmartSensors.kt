package network.hone.app.data

import android.content.Context
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.os.BatteryManager
import org.json.JSONObject
import kotlin.math.abs
import kotlin.math.sqrt

/**
 * Smart phone sensors (Josh 14 Nov: "it aint about turning them off, it is about having them on when there is value").
 *
 * Each sensor runs when its reading is worth something (natural-verifiers doc 2026-11-09, C1-C3, D1), at the
 * lowest rate that keeps that value, and batched in the sensor hub so the phone's CPU sleeps between batches:
 *
 * | Sensor | Value | When on |
 * |---|---|---|
 * | Barometer, humidity, air temp, light | weather and altitude (C1), outdoors/time (D1) | always, 1 Hz, sampled every 10 s by RawRecorder, batched 60 s |
 * | Accelerometer | quakes (C2, MyShake-style: only a still phone can feel them), motion context | still + charging: 20 Hz; still on battery: 10 Hz; moving: 1 Hz (only to notice it stopped) |
 * | Magnetometer | gross place check (C3) | 10 s when the phone is still in a new cell, then off |
 * | Proximity | "in a pocket": light is meaningless then | on change (no polling) |
 * | Gyroscope | none in the background yet | off (the in-app camera evidence uses it itself) |
 *
 * Per 15-min window it adds to the coverage summary: env.pressure_pa / light_lux / rh_pct / air_temp_c (median),
 * env.mag_ut, motion.still_s, motion.shake_events and motion.shake_peak_mg. Shaking is map data only until the
 * quake verifier (USGS, other devices) exists; the barometer is paid by novelty on chain (allow-list).
 */
object SmartSensors : SensorEventListener {

    // ── policy (pure, unit-tested) ─────────────────────────────────────────────────
    data class Ctx(val still: Boolean, val onPower: Boolean, val newPlace: Boolean)
    data class Reg(val sensor: String, val periodUs: Int, val latencyUs: Int)

    fun plan(c: Ctx): List<Reg> = buildList {
        add(when {
            c.still && c.onPower -> Reg("accel", 50_000, 60_000_000)      // 20 Hz: a quake detector on a desk
            c.still -> Reg("accel", 100_000, 300_000_000)                 // 10 Hz, batched 5 min
            else -> Reg("accel", 1_000_000, 60_000_000)                   // 1 Hz: only to notice it stopped
        })
        add(Reg("proximity", 0, 0))                                        // on-change
        if (c.still && c.newPlace) add(Reg("mag", 200_000, 0))           // a 10 s look in a new place
    }

    /** Still = the acceleration magnitude barely moves (phone on a table, a shelf, a car dash at a red light). */
    const val STILL_STD = 0.06          // m/s², std of |a| over a block (OURS)
    /** Shaking felt by a still phone: |a| leaves 1 g by this much (~10 mg; MyShake triggers near here). */
    const val SHAKE_MS2 = 0.10          // OURS
    const val MAG_LOOK_MS = 10_000L

    fun stillBlock(mags: DoubleArray): Boolean {
        if (mags.size < 4) return false
        val m = mags.average()
        return sqrt(mags.sumOf { (it - m) * (it - m) } / mags.size) < STILL_STD
    }

    // ── runtime ─────────────────────────────────────────────────────────────────────
    private var sm: SensorManager? = null
    private var app: Context? = null
    private var active: List<Reg> = emptyList()
    @Volatile private var still = false
    private var stillSinceMs = 0L
    private var lastCell: String? = null
    private var magUntilMs = 0L
    @Volatile private var pocket = false

    // window aggregates
    private val env = HashMap<String, ArrayList<Double>>()
    private val mag = ArrayList<Double>()
    private var stillMs = 0L
    private var shakeEvents = 0
    private var shakePeak = 0.0
    private var inShake = false
    private val block = ArrayList<Double>()
    private var blockStartNs = 0L

    @Synchronized fun start(ctx: Context) {
        app = ctx.applicationContext
        sm = ctx.getSystemService(SensorManager::class.java)
        apply(plan(Ctx(still = false, onPower = onPower(ctx), newPlace = false)))
    }

    @Synchronized fun stop() {
        sm?.unregisterListener(this); active = emptyList()
    }

    /** Called from the sightings tick (every 5 s): re-plans when motion, power or place changed. */
    @Synchronized fun tick(cell: String?) {
        val ctx = app ?: return
        val now = System.currentTimeMillis()
        val newPlace = cell != null && cell != lastCell
        if (newPlace) lastCell = cell
        if (newPlace && still) magUntilMs = now + MAG_LOOK_MS
        val want = plan(Ctx(still, onPower(ctx), now < magUntilMs))
        if (want != active) apply(want)
    }

    private fun apply(want: List<Reg>) {
        val m = sm ?: return
        m.unregisterListener(this)
        for (r in want) {
            val type = when (r.sensor) {
                "accel" -> Sensor.TYPE_ACCELEROMETER; "mag" -> Sensor.TYPE_MAGNETIC_FIELD
                "proximity" -> Sensor.TYPE_PROXIMITY; else -> continue
            }
            val s = m.getDefaultSensor(type) ?: continue
            runCatching {
                if (r.sensor == "proximity") m.registerListener(this, s, SensorManager.SENSOR_DELAY_NORMAL)
                else m.registerListener(this, s, r.periodUs, r.latencyUs)
            }
        }
        active = want
    }

    /** RawRecorder's 10 s climate samples (pressure in Pa, light in lux, humidity, air temperature). */
    @Synchronized fun onEnv(key: String, v: Double) {
        if (key == "env.light_lux" && pocket) return          // a pocket is dark: not a reading of the sky
        env.getOrPut(key) { ArrayList() }.add(v)
    }

    @Synchronized override fun onSensorChanged(e: SensorEvent) {
        when (e.sensor.type) {
            Sensor.TYPE_PROXIMITY -> pocket = e.values[0] < (e.sensor.maximumRange.coerceAtMost(5f))
            Sensor.TYPE_MAGNETIC_FIELD -> {
                mag.add(sqrt((e.values[0] * e.values[0] + e.values[1] * e.values[1] + e.values[2] * e.values[2]).toDouble()))
                if (System.currentTimeMillis() > magUntilMs) tick(lastCell)
            }
            Sensor.TYPE_ACCELEROMETER -> onAccel(e)
        }
    }

    private fun onAccel(e: SensorEvent) {
        val a = sqrt((e.values[0] * e.values[0] + e.values[1] * e.values[1] + e.values[2] * e.values[2]).toDouble())
        if (block.isEmpty()) blockStartNs = e.timestamp
        block.add(a)
        // Shaking only means something on a still phone.
        if (still) {
            val d = abs(a - SensorManager.GRAVITY_EARTH)
            if (d > SHAKE_MS2) {
                if (!inShake) shakeEvents++
                inShake = true
                shakePeak = maxOf(shakePeak, d)
            } else inShake = false
        }
        if (e.timestamp - blockStartNs >= 2_000_000_000L) {        // 2 s blocks (event time: batching-safe)
            val s = stillBlock(block.toDoubleArray())
            val nowMs = System.currentTimeMillis()
            if (s && still) stillMs += (e.timestamp - blockStartNs) / 1_000_000
            if (s != still) { still = s; if (s) stillSinceMs = nowMs; inShake = false }
            block.clear()
        }
    }

    override fun onAccuracyChanged(sensor: Sensor?, accuracy: Int) {}

    /** The window's readings for the coverage summary, then reset. Null when nothing was sampled. */
    @Synchronized fun windowSummary(): Pair<JSONObject?, JSONObject?> {
        fun med(l: List<Double>) = l.sorted()[l.size / 2]
        val envJ = JSONObject()
        for ((k, l) in env) if (l.isNotEmpty()) envJ.put(k.removePrefix("env."), Math.round(med(l) * 100) / 100.0)
        if (mag.isNotEmpty()) envJ.put("mag_ut", Math.round(med(mag) * 10) / 10.0)
        val motion = JSONObject().put("still_s", stillMs / 1000).put("shake_events", shakeEvents)
            .put("shake_peak_mg", Math.round(shakePeak / SensorManager.GRAVITY_EARTH * 1000))
        env.clear(); mag.clear(); stillMs = 0; shakeEvents = 0; shakePeak = 0.0
        return (if (envJ.length() > 0) envJ else null) to motion
    }

    private fun onPower(ctx: Context): Boolean =
        runCatching { ctx.getSystemService(BatteryManager::class.java)?.isCharging }.getOrNull() ?: false
}

package network.hone.app.raw

import android.content.Context
import android.content.pm.PackageManager
import android.hardware.Sensor
import android.hardware.SensorManager

/**
 * Capability-aware sourcing (Josh 2026-11-14 13:52). Each device states what it has; inside a
 * travel group the best source per observation type is picked and the others fill gaps. The
 * Flipper Wi-Fi board is optional, and an owner may have several Flippers.
 *
 *  wifi_env      Flipper with wifi_board, else the phone's own Wi-Fi scans
 *  foot_traffic  wifi_board only (probe requests, channel load); without it a BLE-density
 *                estimate from phone + Flipper, marked precision "low"
 *  location      phone GPS
 * The same obs_type from co-located devices in one window is one reading with corroboration,
 * never two readings (the ingest's products merge them per owner).
 */
object Capabilities {
    data class Dev(val id: String, val cls: String, val caps: Set<String>)
    data class Pick(val obsType: String, val device: String?, val cap: String?, val precision: String)

    /** What this phone has. */
    fun phone(ctx: Context): Set<String> {
        val pm = ctx.packageManager
        val sm = ctx.getSystemService(SensorManager::class.java)
        fun has(f: String) = runCatching { pm.hasSystemFeature(f) }.getOrDefault(false)
        fun sensor(t: Int) = sm?.getDefaultSensor(t) != null
        return buildSet {
            if (has(PackageManager.FEATURE_WIFI)) add("wifi")
            if (has(PackageManager.FEATURE_BLUETOOTH_LE)) add("ble")
            if (has(PackageManager.FEATURE_LOCATION_GPS)) add("gps")
            if (has(PackageManager.FEATURE_TELEPHONY)) add("cell")
            if (has(PackageManager.FEATURE_MICROPHONE)) add("mic")
            if (sensor(Sensor.TYPE_PRESSURE)) add("baro")
            if (sensor(Sensor.TYPE_RELATIVE_HUMIDITY)) add("hygro")
            if (sensor(Sensor.TYPE_AMBIENT_TEMPERATURE)) add("thermo")
            if (sensor(Sensor.TYPE_LIGHT)) add("light")
        }
    }

    /** A Flipper's announced caps in the shared vocabulary ("wifi" in its announce means the dev board). */
    fun flipper(announced: Collection<String>): Set<String> = buildSet {
        add("ble")
        for (c in announced) add(when (c) { "wifi" -> "wifi_board"; "rfid" -> "rfid125"; else -> c })
    }

    /** Capability behind a record key, for provenance. */
    fun capOf(key: String, deviceCaps: Set<String>): String? = when {
        key.startsWith("wifi:") -> if ("wifi_board" in deviceCaps) "wifi_board" else "wifi"
        key.startsWith("ble:") -> "ble"
        key.startsWith("cell:") -> "cell"
        key == "env.pressure_pa" -> "baro"
        key == "env.rh_pct" -> "hygro"
        key == "env.air_temp_c" -> "thermo"
        key == "env.light_lux" -> "light"
        key.startsWith("flipper:") -> "ble"
        key.startsWith("bird:") || key.startsWith("audio") -> "mic"
        key.startsWith("crowd.") -> if ("wifi_board" in deviceCaps) "wifi_board" else "ble"
        key.startsWith("dev.") -> "battery"
        else -> null
    }

    /** Best source per observation type inside a travel group (pure). */
    fun best(obsType: String, group: List<Dev>): Pick {
        fun first(cap: String, cls: String? = null) = group.firstOrNull { cap in it.caps && (cls == null || it.cls == cls) }
        return when (obsType) {
            "wifi_env" -> first("wifi_board")?.let { Pick(obsType, it.id, "wifi_board", "high") }
                ?: first("wifi", "phone")?.let { Pick(obsType, it.id, "wifi", "normal") } ?: Pick(obsType, null, null, "none")
            "foot_traffic" -> first("wifi_board")?.let { Pick(obsType, it.id, "wifi_board", "high") }
                ?: group.firstOrNull { "ble" in it.caps }?.let { Pick(obsType, it.id, "ble", "low") } ?: Pick(obsType, null, null, "none")
            "location" -> first("gps", "phone")?.let { Pick(obsType, it.id, "gps", "high") } ?: Pick(obsType, null, null, "none")
            "subghz" -> first("subghz")?.let { Pick(obsType, it.id, "subghz", "high") } ?: Pick(obsType, null, null, "none")
            "pressure" -> first("baro")?.let { Pick(obsType, it.id, "baro", "high") } ?: Pick(obsType, null, null, "none")
            "cell_coverage" -> first("cell")?.let { Pick(obsType, it.id, "cell", "high") } ?: Pick(obsType, null, null, "none")
            "ble_density" -> first("ble", "phone")?.let { Pick(obsType, it.id, "ble", "normal") }
                ?: first("ble")?.let { Pick(obsType, it.id, "ble", "normal") } ?: Pick(obsType, null, null, "none")
            else -> group.firstOrNull()?.let { Pick(obsType, it.id, null, "normal") } ?: Pick(obsType, null, null, "none")
        }
    }
}

/** Event mode: denser phone scans for a few hours (Android still limits background scans). */
object EventMode {
    private fun p(c: Context) = c.getSharedPreferences("hone_event", Context.MODE_PRIVATE)
    fun until(c: Context): Long = p(c).getLong("until", 0)
    fun active(c: Context): Boolean = System.currentTimeMillis() < until(c)
    fun start(c: Context, hours: Int) = p(c).edit().putLong("until", System.currentTimeMillis() + hours * 3_600_000L).apply()
    fun stop(c: Context) = p(c).edit().putLong("until", 0).apply()
}

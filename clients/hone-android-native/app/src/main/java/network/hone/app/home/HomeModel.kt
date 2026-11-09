package network.hone.app.home

import java.security.MessageDigest
import java.util.Calendar
import java.util.TimeZone
import kotlin.math.asin
import kotlin.math.cos
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * Home inference, pure logic (no Android, unit-tested). Runs on the phone only.
 *
 * Home is where the phone spends its nights: night-time fixes (22:00-06:00 local) that cluster
 * within [RADIUS_M] on at least [MIN_NIGHTS] different nights, and/or a Wi-Fi network the phone is
 * connected to on at least [MIN_NIGHTS] nights. Wi-Fi networks are only ever handled as salted
 * hashes ([wifiHashes]); the SSID and BSSID themselves are never stored.
 */
object HomeModel {
    const val MIN_NIGHTS = 3
    const val RADIUS_M = 150.0
    const val MAX_ACC_M = 150f          // fixes worse than this don't count for the GPS cluster
    const val WIFI_FIX_ACC_M = 500f     // ...but can still place a Wi-Fi-only home
    const val DECLINED_M = 300.0        // never re-suggest within this distance of a "Not home"
    const val NIGHT_START_H = 22
    const val NIGHT_END_H = 6

    /** One sample: when, the fix if any, and salted hashes of the connected Wi-Fi network. */
    data class Sample(val t: Long, val night: String?, val lat: Double?, val lon: Double?, val acc: Float?,
                      val wifi: Set<String> = emptySet())

    data class Candidate(val lat: Double, val lon: Double, val nights: Int, val wifi: Set<String>, val viaWifi: Boolean)

    /** The night a time belongs to (the evening's date, yyyy-mm-dd), or null outside 22:00-06:00 local. */
    fun nightKey(tMs: Long, tz: TimeZone = TimeZone.getDefault()): String? {
        val c = Calendar.getInstance(tz).apply { timeInMillis = tMs }
        val h = c.get(Calendar.HOUR_OF_DAY)
        if (h in NIGHT_END_H until NIGHT_START_H) return null
        if (h < NIGHT_END_H) c.add(Calendar.DAY_OF_MONTH, -1)
        return "%04d-%02d-%02d".format(c.get(Calendar.YEAR), c.get(Calendar.MONTH) + 1, c.get(Calendar.DAY_OF_MONTH))
    }

    /**
     * Salted hashes for a Wi-Fi connection: one for the network name, one for the access point.
     * Placeholders Android returns without permission are ignored. Raw values never leave this call.
     */
    fun wifiHashes(salt: ByteArray, ssid: String?, bssid: String?): Set<String> {
        val out = HashSet<String>()
        val s = ssid?.trim()?.removePrefix("\"")?.removeSuffix("\"")
        if (!s.isNullOrEmpty() && s != "<unknown ssid>") out += "s" + sha(salt, "ssid\u0000$s")
        val b = bssid?.lowercase()
        if (!b.isNullOrEmpty() && b != "02:00:00:00:00:00" && b != "00:00:00:00:00:00") out += "b" + sha(salt, "bssid\u0000$b")
        return out
    }

    private fun sha(salt: ByteArray, v: String): String =
        MessageDigest.getInstance("SHA-256").run { update(salt); update(v.toByteArray(Charsets.UTF_8)); digest() }
            .take(12).joinToString("") { "%02x".format(it) }

    fun distM(lat1: Double, lon1: Double, lat2: Double, lon2: Double): Double {
        val r = 6_371_000.0
        val dLat = Math.toRadians(lat2 - lat1); val dLon = Math.toRadians(lon2 - lon1)
        val a = sin(dLat / 2) * sin(dLat / 2) + cos(Math.toRadians(lat1)) * cos(Math.toRadians(lat2)) * sin(dLon / 2) * sin(dLon / 2)
        return 2 * r * asin(sqrt(a.coerceIn(0.0, 1.0)))
    }

    private fun median(v: List<Double>): Double = v.sorted().let { s -> if (s.size % 2 == 1) s[s.size / 2] else (s[s.size / 2 - 1] + s[s.size / 2]) / 2 }

    /**
     * Best home candidate, or null. [declined] are places the user said are not home;
     * [declinedWifi] hashes of networks they said are not home Wi-Fi.
     */
    fun infer(samples: List<Sample>, declined: List<Pair<Double, Double>> = emptyList(),
              declinedWifi: Set<String> = emptySet()): Candidate? {
        val night = samples.filter { it.night != null }
        fun isDeclined(lat: Double, lon: Double) = declined.any { distM(lat, lon, it.first, it.second) <= DECLINED_M }

        // Wi-Fi networks connected on enough nights.
        val wifiNights = HashMap<String, MutableSet<String>>()
        night.forEach { s -> s.wifi.forEach { h -> wifiNights.getOrPut(h) { HashSet() }.add(s.night!!) } }
        val homeWifi = wifiNights.filter { it.value.size >= MIN_NIGHTS && it.key !in declinedWifi }.keys

        // GPS: the night fix whose neighbourhood spans the most nights.
        val fixes = night.filter { it.lat != null && it.lon != null && (it.acc ?: Float.MAX_VALUE) <= MAX_ACC_M }
        var best: Pair<List<Sample>, Int>? = null
        for (s in fixes) {
            val near = fixes.filter { distM(s.lat!!, s.lon!!, it.lat!!, it.lon!!) <= RADIUS_M }
            val n = near.mapTo(HashSet()) { it.night }.size
            if (n >= MIN_NIGHTS && (best == null || n > best.second || (n == best.second && near.size > best.first.size))) {
                val lat = median(near.map { it.lat!! }); val lon = median(near.map { it.lon!! })
                if (!isDeclined(lat, lon)) best = near to n
            }
        }
        if (best != null) {
            val (near, n) = best
            val lat = median(near.map { it.lat!! }); val lon = median(near.map { it.lon!! })
            // Home Wi-Fi: networks seen on enough nights that were connected here.
            val here = samples.filter { it.lat != null && distM(lat, lon, it.lat, it.lon!!) <= RADIUS_M * 2 }.flatMapTo(HashSet()) { it.wifi }
            return Candidate(lat, lon, n, homeWifi intersect here, viaWifi = false)
        }
        // Wi-Fi only: the most-used night network, placed by whatever fixes came with it.
        val top = homeWifi.maxByOrNull { wifiNights.getValue(it).size } ?: return null
        val placed = samples.filter { top in it.wifi && it.lat != null && (it.acc ?: Float.MAX_VALUE) <= WIFI_FIX_ACC_M }
        if (placed.isEmpty()) return null
        val lat = median(placed.map { it.lat!! }); val lon = median(placed.map { it.lon!! })
        if (isDeclined(lat, lon)) return null
        // Other hashes of the same connection (name + access point) come along.
        val same = homeWifi.filter { h -> night.any { top in it.wifi && h in it.wifi } }.toSet()
        return Candidate(lat, lon, wifiNights.getValue(top).size, same, viaWifi = true)
    }

    /** True when [current] (hashes of the connected network) matches the remembered home Wi-Fi. */
    fun onHomeWifi(current: Set<String>, home: Set<String>): Boolean = current.isNotEmpty() && current.any { it in home }
}

package network.hone.app.birds

import android.content.Context
import network.hone.app.sightings.Cells
import org.json.JSONObject
import java.security.MessageDigest
import java.time.Instant
import java.time.ZoneId

/**
 * Spots and Keepers. A spot is a public res-9 block (home reports are only ever shared as a 5 km
 * area, so homes never become spots). Its Keeper is whoever listened there on the most mornings
 * times the most kinds over 30 days. A challenge costs 20 energy and runs 7 days; only calls heard
 * at the spot with a close location fix (50 m) count for the challenger.
 */
object Spots {
    data class Spot(val cell: String, val lat: Double, val lon: Double, val kinds: Int, val listeners: Int,
                    val keeper: String?, val keeperIsMe: Boolean, val keeperMornings: Int, val keeperKinds: Int)

    data class Mine(val mornings: Int, val kinds: Int) { val score get() = mornings * kinds }

    private const val CLOSE_FIX_M = 50.0
    private const val DAY_MS = 86_400_000L
    const val CHALLENGE_DAYS = 7

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_spots", Context.MODE_PRIVATE)

    private fun hash(s: String) = MessageDigest.getInstance("SHA-256").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }.take(12)

    /** Spots in the phone's res-7 area, from the indexer. Null when offline or without a location. Off the main thread. */
    fun near(ctx: Context, lat: Double, lon: Double): List<Spot>? {
        val c7 = Cells.cell(lat, lon, 7) ?: return null
        val me = BirdsSync.account(ctx)?.let { hash(it) }
        val j = runCatching { JSONObject(BirdsSync.getJson("/spots?cell=$c7")) }.getOrNull() ?: return null
        val a = j.getJSONArray("spots")
        return (0 until a.length()).map { a.getJSONObject(it) }.map {
            Spot(it.getString("spot"), it.getDouble("lat"), it.getDouble("lon"), it.getInt("kinds"), it.getInt("listeners"),
                if (it.isNull("keeper")) null else it.getString("keeper"), me != null && it.optString("keeper_hash") == me,
                it.getInt("keeper_mornings"), it.getInt("keeper_kinds"))
        }
    }

    /** This phone's own mornings and kinds at [cell] over the last 30 days (close fixes only). */
    fun mine(ctx: Context, cell: String, since: Long = System.currentTimeMillis() - 30 * DAY_MS): Mine {
        val z = ZoneId.systemDefault()
        val days = HashSet<Long>(); val kinds = HashSet<String>()
        Birds.heard(ctx).filter { it.t >= since && it.lat != null && it.lon != null && Birds.counts(ctx, it) }.forEach { h ->
            if ((h.acc ?: 999.0) > CLOSE_FIX_M) return@forEach
            if (Cells.cell(h.lat!!, h.lon!!, 9) != cell) return@forEach
            kinds += h.name
            val local = Instant.ofEpochMilli(h.t).atZone(z)
            if (local.hour in 4..9) days += local.toLocalDate().toEpochDay()
        }
        return Mine(days.size, kinds.size)
    }

    // ── challenges ──────────────────────────────────────────────────────────────
    fun challengeEnds(ctx: Context, cell: String): Long = prefs(ctx).getLong("ends_$cell", 0L)

    fun challenge(ctx: Context, cell: String): Boolean {
        if (System.currentTimeMillis() < challengeEnds(ctx, cell)) return true
        if (!Energy.spend(ctx, "challenge:$cell", Energy.CHALLENGE)) return false
        prefs(ctx).edit().putLong("ends_$cell", System.currentTimeMillis() + CHALLENGE_DAYS * DAY_MS).apply()
        return true
    }
}

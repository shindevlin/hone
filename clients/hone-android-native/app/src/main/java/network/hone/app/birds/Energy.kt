package network.hone.app.birds

import android.content.Context
import network.hone.app.sound.SoundSensing
import org.json.JSONArray
import org.json.JSONObject

/**
 * Energy spending. Earning is computed in Today (listening, new blocks, honest answers, goals);
 * spends are kept here, on the phone. Energy can't be bought, sold or traded.
 */
object Energy {
    const val FOCUS = 10        // Focus mode: listen every frame for an hour
    const val NEARBY = 5        // Who's nearby: five birds likely around you this week
    const val CHALLENGE = 20    // Challenge a spot's Keeper for 7 days
    const val FOCUS_MS = 3600_000L

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("hone_energy", Context.MODE_PRIVATE)

    fun spent(ctx: Context): Int = runCatching {
        val a = JSONArray(prefs(ctx).getString("spent", "[]"))
        (0 until a.length()).sumOf { a.getJSONObject(it).getInt("cost") }
    }.getOrDefault(0)

    fun available(ctx: Context): Int = (Today.compute(ctx).energy - spent(ctx)).coerceAtLeast(0)

    /** Spend [cost] for [what]; false (nothing spent) when there isn't enough. */
    @Synchronized
    fun spend(ctx: Context, what: String, cost: Int): Boolean {
        if (available(ctx) < cost) return false
        val a = runCatching { JSONArray(prefs(ctx).getString("spent", "[]")) }.getOrDefault(JSONArray())
        a.put(JSONObject().put("t", System.currentTimeMillis()).put("what", what).put("cost", cost))
        prefs(ctx).edit().putString("spent", a.toString()).apply()
        return true
    }

    // ── Focus mode ──────────────────────────────────────────────────────────────
    fun focusUntil(ctx: Context) = prefs(ctx).getLong("focus_until", 0L)

    fun startFocus(ctx: Context): Boolean {
        if (System.currentTimeMillis() < focusUntil(ctx)) return true
        if (!spend(ctx, "focus", FOCUS)) return false
        val until = System.currentTimeMillis() + FOCUS_MS
        prefs(ctx).edit().putLong("focus_until", until).apply()
        SoundSensing.paidFocusUntilMs = until
        return true
    }

    /** After a restart: carry on with focus already paid for. */
    fun restore(ctx: Context) { SoundSensing.paidFocusUntilMs = focusUntil(ctx) }

    // ── Who's nearby ────────────────────────────────────────────────────────────
    /** Five birds likely around here this week (shared area scores); charges only when there is an answer. */
    fun nearby(ctx: Context): List<BirdsSync.Around>? {
        val cached = prefs(ctx).getString("nearby", null)?.let { s ->
            runCatching {
                val j = JSONObject(s)
                if (System.currentTimeMillis() - j.getLong("t") > 3600_000L) null
                else j.getJSONArray("rows").let { a -> (0 until a.length()).map { a.getJSONObject(it) }
                    .map { BirdsSync.Around(it.getString("latin"), it.getString("status"), it.getDouble("p"), it.optInt("listeners")) } }
            }.getOrNull()
        }
        if (cached != null) return cached
        val rows = BirdsSync.around(ctx)?.filter { it.status != "No longer heard" }?.sortedByDescending { it.p }?.take(5) ?: return null
        if (rows.isEmpty() || !spend(ctx, "nearby", NEARBY)) return if (rows.isEmpty()) rows else null
        prefs(ctx).edit().putString("nearby", JSONObject().put("t", System.currentTimeMillis()).put("rows", JSONArray(rows.map {
            JSONObject().put("latin", it.latin).put("status", it.status).put("p", it.p).put("listeners", it.listeners) })).toString()).apply()
        return rows
    }
}

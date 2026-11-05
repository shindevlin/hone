package network.hone.app.birds

import android.content.Context
import network.hone.app.sightings.Cells
import org.json.JSONObject
import java.io.File
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.temporal.IsoFields

/** One of the three daily goals. */
data class Goal(val title: String, val progress: Int, val target: Int, val energy: Int) {
    val done get() = progress >= target
}

/** Ornigo "Today": streak, three goals, energy. Computed from the local logs only. */
data class TodayStats(
    val listenedMinToday: Int,
    val birdsToday: Int,
    val newForYou: Int,
    val cameBack: Int,
    val streak: Int,
    /** Last 7 days, oldest first: 'y' kept, 'f' kept with the week's freeze, 'n' missed, 't' today still open. */
    val week: List<Char>,
    val freezeLeftThisWeek: Boolean,
    val goals: List<Goal>,
    val energy: Int,
)

object Today {
    const val STREAK_SEC = 600          // 10 real minutes keeps the streak
    private const val WINDOW_MS = 900_000L

    private fun day(t: Long, z: ZoneId): LocalDate = Instant.ofEpochMilli(t).atZone(z).toLocalDate()

    fun compute(ctx: Context): TodayStats {
        val z = ZoneId.systemDefault()
        val today = LocalDate.now(z)
        val dir = File(ctx.filesDir, "hone-node/sound")

        // Listening per day, from the 15-minute window reports.
        val listened = HashMap<LocalDate, Int>()
        File(dir, "reports.jsonl").takeIf { it.exists() }?.forEachLine { l ->
            runCatching {
                val j = JSONObject(l)
                val d = day(j.getLong("window") * WINDOW_MS, z)
                listened[d] = (listened[d] ?: 0) + j.optInt("listen_seconds")
            }
        }

        val heard = Birds.heard(ctx).filter { it.label != false }
        val firstBySpecies = HashMap<String, Long>()
        heard.forEach { h -> firstBySpecies[h.name] = minOf(firstBySpecies[h.name] ?: Long.MAX_VALUE, h.t) }
        val todayHeard = heard.filter { day(it.t, z) == today }
        val todaySpecies = todayHeard.map { it.name }.toSet()
        val newForYou = todaySpecies.count { day(firstBySpecies[it]!!, z) == today }
        val cameBack = todaySpecies.count { sp ->
            val before = heard.filter { it.name == sp && day(it.t, z) < today }.maxOfOrNull { it.t } ?: return@count false
            today.toEpochDay() - day(before, z).toEpochDay() > 7
        }

        // Labels answered today ("honest labels").
        var labelsToday = 0; var labelsAll = 0
        File(dir, "labels.jsonl").takeIf { it.exists() }?.forEachLine { l ->
            runCatching { JSONObject(l).getLong("at") }.getOrNull()?.let { at -> labelsAll++; if (day(at, z) == today) labelsToday++ }
        }

        // Streak: count back from yesterday (today counts once it reaches 10 minutes); one freeze per ISO week.
        fun kept(d: LocalDate) = (listened[d] ?: 0) >= STREAK_SEC
        val freezeUsed = HashSet<Int>()
        fun weekKey(d: LocalDate) = d.get(IsoFields.WEEK_BASED_YEAR) * 100 + d.get(IsoFields.WEEK_OF_WEEK_BASED_YEAR)
        val status = HashMap<LocalDate, Char>()
        var streak = if (kept(today)) 1 else 0
        var d = today.minusDays(1)
        while (true) {
            if (kept(d)) { streak++; status[d] = 'y' }
            else if (streak > 0 && weekKey(d) !in freezeUsed && kept(d.minusDays(1))) { freezeUsed += weekKey(d); status[d] = 'f' }
            else break
            d = d.minusDays(1)
            if (streak > 3650) break
        }
        val week = (6 downTo 0).map { back ->
            val x = today.minusDays(back.toLong())
            when {
                back == 0 -> if (kept(today)) 'y' else 't'
                status[x] != null -> status[x]!!
                kept(x) -> 'y'
                else -> 'n'
            }
        }

        val minToday = (listened[today] ?: 0) / 60
        val goals = listOf(
            Goal("Listen for 10 minutes", minToday.coerceAtMost(10), 10, 10),
            Goal("Hear 3 kinds of birds", todaySpecies.size.coerceAtMost(3), 3, 10),
            Goal("Answer one \"Is this right?\"", labelsToday.coerceAtMost(1), 1, 5),
        )

        // Energy: live listening, new blocks, honest labels, finished goals. Not for sale, not tradable.
        val listenEnergy = listened.values.sum() / 300
        val blocks = heard.mapNotNull { h -> if (h.lat != null && h.lon != null) Cells.cell(h.lat, h.lon, 9) ?: "%.3f,%.3f".format(h.lat, h.lon) else null }.toSet().size
        val goalsEnergy = goals.filter { it.done }.sumOf { it.energy }
        val energy = listenEnergy + 5 * blocks + 2 * labelsAll + goalsEnergy

        return TodayStats(minToday, todaySpecies.size, newForYou, cameBack, streak, week,
            weekKey(today) !in freezeUsed, goals, energy)
    }
}

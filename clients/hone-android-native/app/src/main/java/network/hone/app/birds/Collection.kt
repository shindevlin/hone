package network.hone.app.birds

import android.content.Context

/**
 * Ornigo collection rules, from what this phone and the shared area scores know.
 *  - Rarity pips (1-3): Regular here = 1, listed in this area but not regular = 2, not listed = 3.
 *  - Shiny: confirmed by your photo, or by a second listener in this area, or you were first in
 *    this block (the indexer said so when your report went up).
 *  - Legendary (a known individual) stays hidden until the voiceprint shadow test passes.
 */
object Collection {
    enum class Shiny { PHOTO, SECOND_LISTENER, FIRST_IN_BLOCK }

    data class Card(val pips: Int, val shiny: Shiny?, val caught: Int)

    fun card(ctx: Context, latin: String, around: List<BirdsSync.Around>?): Card {
        val a = around?.firstOrNull { it.latin == latin }
        val pips = when {
            around == null -> 0                 // offline: no pips rather than a guess
            a == null -> 3
            a.status == "Regular here" -> 1
            else -> 2
        }
        val shiny = when {
            Birds.myPhotos(ctx, latin).any { it.status != "rejected" } -> Shiny.PHOTO
            (a?.listeners ?: 0) >= 2 -> Shiny.SECOND_LISTENER
            network.hone.app.engage.Engagement.wasFirstInArea(ctx, latin) -> Shiny.FIRST_IN_BLOCK
            else -> null
        }
        return Card(pips, shiny, Catches.count(ctx, latin))
    }

    /** Times heard per week for the last 12 weeks, oldest first (index 11 = this week). */
    fun weeks(history: List<Heard>, now: Long = System.currentTimeMillis()): IntArray {
        val out = IntArray(12)
        val week = 7 * 86_400_000L
        for (h in history) {
            val ago = ((now - h.t) / week).toInt()
            if (ago in 0..11) out[11 - ago]++
        }
        return out
    }
}

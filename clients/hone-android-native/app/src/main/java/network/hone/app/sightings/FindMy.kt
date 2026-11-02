package network.hone.app.sightings

import android.content.Context

/** One sighting of an owned device, resolved back to the device it belongs to. */
data class OwnedSighting(
    val device: String, val window: Long, val cell9: String?, val cellf: String?,
    val rssi: Int, val ts: Long, val source: String,
)

/**
 * Where sightings come from. Today only this phone's own store; later a chain indexer that
 * holds everyone's (rid, encrypted cell, window, rssi) — same query shape: the owner sends
 * the rids it recomputed from its seeds and gets back the sightings for them.
 */
interface SightingIndex {
    val name: String
    fun lookup(rids: Collection<String>): List<SightingStore.Sighting>
}

class LocalSightingIndex(private val store: SightingStore) : SightingIndex {
    override val name = "this phone"
    override fun lookup(rids: Collection<String>) = store.ridSightings(rids)
}

// TODO(indexer): class ChainSightingIndex : SightingIndex — POST the rid list to the
// indexer, decrypt each cell with the owner key (spec §1a, Find My model).

/**
 * "Find my things": recompute every rid each owned seed used over the last [days]
 * (96 HMACs per device per day) and ask each index for sightings of them.
 */
object FindMy {
    fun ownedSeeds(ctx: Context, store: SightingStore): List<Pair<String, ByteArray>> =
        listOf("This phone" to Rid.seed(ctx)) +
            store.ownedDevices().mapNotNull { (label, hex) ->
                runCatching { hex.unhex() }.getOrNull()?.takeIf { it.size == 32 }?.let { label to it }
            }

    fun search(ctx: Context, store: SightingStore, indexes: List<SightingIndex>, days: Int = 7): List<OwnedSighting> {
        val now = Rid.window()
        val out = ArrayList<OwnedSighting>()
        for ((label, seed) in ownedSeeds(ctx, store)) {
            val rids = HashMap<String, Long>()
            for (w in (now - days * 96L)..now) rids[Rid.rid(seed, w).hex()] = w
            for (ix in indexes) for (s in ix.lookup(rids.keys)) {
                out += OwnedSighting(label, s.window, s.cell9, s.cellf, s.rssi, s.ts, ix.name)
            }
        }
        return out.sortedByDescending { it.ts }
    }
}

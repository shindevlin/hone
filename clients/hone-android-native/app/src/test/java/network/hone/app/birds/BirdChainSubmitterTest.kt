package network.hone.app.birds

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class BirdChainSubmitterTest {
    private val c9 = "896d75d960bffff"
    private val c7 = "876d75d96ffffff"
    private fun row(cell: String, w: Long, m: String, v: Long = 1) = Triple(cell, w, m to v)

    @Test fun groupsRowsByChainCellAndWindow() {
        val rows = listOf(row(c9, 10, "bird.sp:turdus_grayi"), row(c9, 10, "bird.gr:ducks"), row(c9, 11, "bird.sp:turdus_grayi"))
        val e = BirdChainSubmitter.entries(rows) { it }
        assertEquals(2, e.size)
        assertEquals(mapOf("bird.gr:ducks" to 1L, "bird.sp:turdus_grayi" to 1L), e[0].metrics)
        assertEquals(11L, e[1].window)
    }

    @Test fun coarsensToTheChainCellAndKeepsTheHighestValue() {
        val rows = listOf(row(c9, 10, "bird.sp:aix_sponsa", 1), row("896d75d960fffff", 10, "bird.sp:aix_sponsa", 2))
        val e = BirdChainSubmitter.entries(rows) { c7 }
        assertEquals(listOf(BirdChainSubmitter.Entry(c7, 10, mapOf("bird.sp:aix_sponsa" to 2L))), e)
    }

    @Test fun anUpgradeReplacesTheSameSpeciesInTheSameEntry() {
        val rows = listOf(row(c9, 10, "bird.gr:ducks"), row(c9, 10, "bird.up:anas_acuta"), row(c9, 10, "bird.sp:anas_acuta"))
        val m = BirdChainSubmitter.entries(rows) { it }.single().metrics
        assertEquals(setOf("bird.gr:ducks", "bird.up:anas_acuta"), m.keys)
    }

    @Test fun dropsMapOnlyCellsBadNamesAndValues() {
        val rows = listOf(row(c9, 10, "bird.sp:Turdus grayi"), row(c9, 10, "wifi.ap_count"), row(c9, 10, "bird.sp:turdus_grayi", 7),
            row("856d75d9fffffff", 10, "bird.sp:turdus_grayi"))
        assertTrue(BirdChainSubmitter.entries(rows) { if (it == c9) it else null }.isEmpty())
    }

    @Test fun splitsAtThirtyTwoMetrics() {
        val rows = (0 until 40).map { row(c9, 10, "bird.sp:genus_s" + "abcdefghijklmnopqrstuvwxyzabcdefghijklmn"[it] + it.toString().map { d -> 'a' + (d - '0') }.joinToString("")) }
        val e = BirdChainSubmitter.entries(rows) { it }
        assertEquals(listOf(32, 8), e.map { it.metrics.size })
    }

    @Test fun gateIsClosedUntilTheChainSaysSo() {
        assertFalse(BirdChainSubmitter.gateOpen(380_000, -1))
        assertFalse(BirdChainSubmitter.gateOpen(380_000, Long.MAX_VALUE))
        assertFalse(BirdChainSubmitter.gateOpen(380_000, 380_001))
        assertTrue(BirdChainSubmitter.gateOpen(380_000, 380_000))
    }
}

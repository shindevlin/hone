package network.hone.app.data

import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/**
 * A UI-facing snapshot of node state. Mirrors the Rust `NodeStatus` uniffi
 * record (running / statusText / epoch / balanceHunits) plus a few UI-only
 * derived fields. When the uniffi bridge lands (Phase 0b), this maps 1:1 from
 * the generated `uniffi.hone_miner.NodeStatus`.
 */
data class NodeSnapshot(
    val running: Boolean,
    val statusText: String,
    val epoch: ULong,
    val balanceHunits: ULong,
    val peers: Int,
    val isMiner: Boolean,
    val isClock: Boolean,
) {
    /** 1 HONE = 10^10 hunits (canonical HUNITS_PER_HONE). */
    val balanceHone: Double get() = balanceHunits.toDouble() / 10_000_000_000.0
}

/**
 * The single seam between UI and the Rust core. Screens/ViewModels depend on
 * THIS interface, never on the bridge directly — so the app builds and runs
 * with [MockNodeRepository] today (no NDK), and swaps to the real bridge-backed
 * impl in Phase 0b with zero UI changes.
 */
interface NodeRepository {
    /** A hot stream of node state for the UI to collect. */
    fun observe(): Flow<NodeSnapshot>
    suspend fun start(isMiner: Boolean, isClock: Boolean)
    suspend fun stop()
    suspend fun balanceOf(account: String): ULong
}

/**
 * Mock implementation — deterministic-ish fake data so the entire Compose UI is
 * buildable and demo-able before the native `.so` exists. Emits a ticking epoch
 * and slowly-growing balance to make the dashboard feel alive.
 *
 * Phase 0b replaces this with `BridgeNodeRepository` wrapping `HoneNode` from
 * the generated uniffi bindings. The `observe()` contract stays identical.
 */
class MockNodeRepository : NodeRepository {
    @Volatile private var running = false
    @Volatile private var miner = true
    @Volatile private var clock = true

    override fun observe(): Flow<NodeSnapshot> = flow {
        var epoch = 2963UL
        var balance = 1_250_000_000UL // 0.125 HONE
        while (true) {
            if (running) {
                epoch += 1UL
                if (miner) balance += 4_200_000UL // fake reward drip
            }
            emit(
                NodeSnapshot(
                    running = running,
                    statusText = if (running) "Sealing epochs" else "Stopped",
                    epoch = epoch,
                    balanceHunits = balance,
                    peers = if (running) 13 else 0,
                    isMiner = miner,
                    isClock = clock,
                )
            )
            delay(2_000)
        }
    }

    override suspend fun start(isMiner: Boolean, isClock: Boolean) {
        miner = isMiner; clock = isClock; running = true
    }

    override suspend fun stop() { running = false }

    override suspend fun balanceOf(account: String): ULong = 0UL
}

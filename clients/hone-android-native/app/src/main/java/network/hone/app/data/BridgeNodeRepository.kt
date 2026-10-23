package network.hone.app.data

import android.content.Context
import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOn
import kotlinx.coroutines.withContext
import uniffi.hone_miner.HoneNode
import uniffi.hone_miner.NodeConfig

/**
 * Live-chain parameters for chain "hone" (30 s epochs, genesis 2026-05-01).
 * Bootstrap is grouchly only: it is the sturdiest LAN node, and nebra has a
 * known gossip-reconnect bug we do not want a roaming phone to poke.
 */
object LiveChain {
    const val CHAIN_ID = "hone"
    const val GENESIS_TS_MS = 1_783_191_600_000L
    val BOOTSTRAP = listOf(
        "/ip4/192.168.68.72/tcp/6942/p2p/12D3KooWJSS73VUJwKHRfgEZHRkkhRFs6HwqPn3BoPkEAipmDTNb",
    )
    const val P2P_PORT = 6943
}

/**
 * Phase 0b: the real Rust core behind the [NodeRepository] seam.
 *
 * OBSERVER ONLY for now: it starts the node with no account and no posting key,
 * which the Rust core treats as read-only (it listens to gossip and drops every
 * outbound publish; no miner, inference worker or verifier). Miner/clock roles
 * need the phone's own on-chain account + stake first.
 */
class BridgeNodeRepository(private val context: Context) : NodeRepository {
    private val node = HoneNode()

    override fun observe(): Flow<NodeSnapshot> = flow {
        while (true) {
            val s = node.status()
            emit(
                NodeSnapshot(
                    running = s.running,
                    statusText = s.statusText,
                    epoch = s.epoch,
                    balanceHunits = s.balanceHunits,
                    peers = s.peers.toInt(),
                    isMiner = false,
                    isClock = false,
                )
            )
            delay(2_000)
        }
    }.flowOn(Dispatchers.IO)

    override suspend fun start(isMiner: Boolean, isClock: Boolean) {
        withContext(Dispatchers.IO) {
            val cfg = NodeConfig(
                account = "",
                postingKey = "",
                chainId = LiveChain.CHAIN_ID,
                genesisTs = LiveChain.GENESIS_TS_MS.toULong(),
                dataDir = context.filesDir.resolve("hone-node").absolutePath,
                modelId = "",
                modelDir = context.filesDir.resolve("models").absolutePath,
                bootstrapPeers = LiveChain.BOOTSTRAP,
                p2pPort = LiveChain.P2P_PORT.toUShort(),
                isMiner = false,
                isClock = false,
            )
            runCatching { node.start(cfg) }
                .onFailure { Log.e("HoneNode", "start failed", it) }
        }
    }

    override suspend fun stop() {
        withContext(Dispatchers.IO) { runCatching { node.stop() } }
    }

    override suspend fun balanceOf(account: String): ULong =
        withContext(Dispatchers.IO) { node.balanceOf(account) }
}

/** One node per process: survives ViewModel/Activity recreation. */
object NodeRepositories {
    @Volatile private var bridge: BridgeNodeRepository? = null

    fun get(context: Context): NodeRepository =
        bridge ?: synchronized(this) {
            bridge ?: BridgeNodeRepository(context.applicationContext).also { bridge = it }
        }
}

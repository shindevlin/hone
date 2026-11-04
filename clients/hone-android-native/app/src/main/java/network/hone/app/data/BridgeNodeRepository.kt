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
        // The embedded core is a light seat and does not hold the ledger, so the
        // balance comes from the chain over HTTP (the seat's account earns its rewards).
        val account = runCatching {
            context.filesDir.resolve("hone-node/witness_account").readText().trim()
        }.getOrDefault("")
        var chainBalance: ULong? = null
        var tick = 0
        while (true) {
            if (account.isNotEmpty() && tick % 15 == 0) {
                ChainHttp.balanceHunits(account)?.let { chainBalance = it }
            }
            tick++
            val s = node.status()
            emit(
                NodeSnapshot(
                    running = s.running,
                    statusText = s.statusText,
                    epoch = s.epoch,
                    balanceHunits = chainBalance ?: s.balanceHunits,
                    peers = s.peers.toInt(),
                    isMiner = false,
                    isClock = s.witness,
                )
            )
            delay(2_000)
        }
    }.flowOn(Dispatchers.IO)

    override suspend fun start(isMiner: Boolean, isClock: Boolean) {
        withContext(Dispatchers.IO) {
            // Witness clock: if hone-node/witness_account names an account (e.g. "josh"),
            // the core runs as a witness clock seat of that account: node id
            // "josh@<device pubkey>", device key in hone-node/witness.key, seals only.
            // The seat counts toward quorum once the account stakes on it.
            // Without the file it stays a read-only observer.
            val dataDir = context.filesDir.resolve("hone-node").apply { mkdirs() }
            val witnessAccount = runCatching {
                dataDir.resolve("witness_account").readText().trim()
            }.getOrDefault("")
            val cfg = NodeConfig(
                account = witnessAccount,
                postingKey = "",
                chainId = LiveChain.CHAIN_ID,
                genesisTs = LiveChain.GENESIS_TS_MS.toULong(),
                dataDir = dataDir.absolutePath,
                modelId = "",
                modelDir = context.filesDir.resolve("models").absolutePath,
                bootstrapPeers = LiveChain.BOOTSTRAP,
                p2pPort = LiveChain.P2P_PORT.toUShort(),
                isMiner = false,
                isClock = witnessAccount.isNotEmpty(),
            )
            runCatching { node.start(cfg) }
                .onFailure { Log.e("HoneNode", "start failed", it) }
        }
    }

    /** Start the seat if it is not running (NodeService calls this after boot/update). */
    suspend fun ensureStarted() {
        val running = withContext(Dispatchers.IO) { runCatching { node.status().running }.getOrDefault(true) }
        if (!running) start(isMiner = false, isClock = true)
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

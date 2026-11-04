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
 * Bootstrap: the public seed at honemesh.net first, so any phone anywhere reaches
 * the mesh; a home LAN node is only a fallback. nebra is left out: it has a known
 * gossip-reconnect bug we do not want a roaming phone to poke.
 */
object LiveChain {
    const val CHAIN_ID = "hone"
    const val GENESIS_TS_MS = 1_783_191_600_000L
    val BOOTSTRAP = listOf(
        // Public seed (ferryvps, non-clock relay node): reachable from anywhere.
        "/dns4/honemesh.net/tcp/6942/p2p/12D3KooWQnJPfAYbktWXv1zFoGCQAz4v7qWthLTdrD7ey5JPDCUt",
        "/ip4/162.35.10.46/tcp/6942/p2p/12D3KooWQnJPfAYbktWXv1zFoGCQAz4v7qWthLTdrD7ey5JPDCUt",
        // Home LAN (optional fallback; mDNS finds LAN peers anyway).
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
        // Never a local counter: the embedded core's own balance/epoch can disagree with the chain.
        var chainBalance: ULong? = null
        var chainEpoch: ULong? = null
        var sealed: Long? = null
        var account = ""
        var tick = 0
        while (true) {
            if (tick % 15 == 0) {
                account = runCatching {
                    context.filesDir.resolve("hone-node/witness_account").readText().trim()
                }.getOrDefault("")
                chainBalance = if (account.isEmpty()) null else ChainHttp.balanceHunits(account) ?: chainBalance
                ChainHttp.get("/api/node/info")?.let { b ->
                    runCatching { org.json.JSONObject(b).getLong("epoch").toULong() }.getOrNull()?.let { chainEpoch = it }
                }
                sealed = if (account.isEmpty()) null else sealedToday("$account@${seatPub()}") ?: sealed
            }
            tick++
            val s = node.status()
            emit(
                NodeSnapshot(
                    running = s.running,
                    statusText = s.statusText,
                    epoch = chainEpoch,
                    balanceHunits = chainBalance,
                    peers = s.peers.toInt(),
                    isMiner = false,
                    isClock = s.witness,
                    seat = if (account.isEmpty()) null else "$account@${seatPub().take(4)}…",
                    sealedToday = sealed,
                )
            )
            delay(2_000)
        }
    }.flowOn(Dispatchers.IO)

    private fun seatPub(): String =
        network.hone.app.sightings.RoleKeys.devicePub(context).joinToString("") { "%02x".format(it) }

    /** Sealed epochs today for this seat, counted by the indexer from finalized epochs' sealed_by. */
    private fun sealedToday(nodeId: String): Long? = runCatching {
        val c = java.net.URL("https://honemesh.net/api/devices/seals/" + java.net.URLEncoder.encode(nodeId, "UTF-8"))
            .openConnection() as java.net.HttpURLConnection
        c.connectTimeout = 5_000; c.readTimeout = 8_000
        try {
            if (c.responseCode != 200) null
            else org.json.JSONObject(c.inputStream.bufferedReader().use { it.readText() }).getLong("sealed_today")
        } finally { c.disconnect() }
    }.getOrNull()

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

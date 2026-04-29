/*!
# btcpc-node

BTCPC sovereign chain node — single binary for networking, consensus,
state machine, block production, contract execution, and HTTP API.

## Environment Variables

    BTCPC_DATA_DIR        — RocksDB data directory (default: ~/.btcpc)
    BTCPC_ACCOUNT         — this node's account name
    BTCPC_NODE_ID         — libp2p node identity label
    BTCPC_API_PORT        — HTTP API port (default: 4242)
    BTCPC_P2P_PORT        — libp2p listen port (default: 6942)
    BTCPC_MINER           — "true" to enable mining
    BTCPC_CLOCK           — "true" to participate in clock consensus
    BTCPC_GENESIS_FILE    — path to genesis.json
    BTCPC_LOG_LEVEL       — tracing filter (default: btcpc_node=info)
    BTCPC_BOOTSTRAP_PEERS — comma-separated multiaddrs for DHT bootstrap
*/

mod api;
mod chain;
mod clock;
mod config;
mod contracts;
mod finalize;
mod genesis;
mod miner;
mod net;
mod store;
mod tx;

use std::sync::Arc;
use anyhow::Result;
use tracing::info;

use chain::Chain;
use config::Config;
use contracts::ContractEngine;
use store::Store;

#[tokio::main]
async fn main() -> Result<()> {
    let cfg = Config::from_env();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| cfg.log_level.parse().unwrap_or_default()),
        )
        .init();

    info!("btcpc-node starting — account={} data={:?}", cfg.account, cfg.data_dir);

    // Open state database
    let db_path = cfg.data_dir.join("state");
    let store = Store::open(&db_path)?;
    let chain = Arc::new(Chain::new(store, cfg.node_id.clone()));

    // Genesis
    genesis::init_genesis(&chain, cfg.genesis_file.as_deref())?;

    info!("chain state ready — latest epoch={}", chain.current_epoch());

    // ── Clock consensus ───────────────────────────────────────────────────────
    let clock = Arc::new(clock::ClockConsensus::new());
    {
        let clock_ref = clock.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(tokio::time::Duration::from_millis(1_000)).await;
                clock_ref.tick();
            }
        });
    }

    // ── Finalizer ─────────────────────────────────────────────────────────────
    {
        let chain_ref = chain.clone();
        tokio::spawn(async move {
            finalize::run_finalizer(chain_ref, 10).await;
        });
    }

    // ── Mining ────────────────────────────────────────────────────────────────
    if cfg.is_miner {
        let chain_ref = chain.clone();
        let account = cfg.account.clone();
        tokio::spawn(async move {
            miner::run_miner(chain_ref, account).await;
        });
    }

    // ── Networking ────────────────────────────────────────────────────────────
    let (network, _net_handle, mut _net_events) = net::Network::new(cfg.clone());
    tokio::spawn(async move {
        if let Err(e) = network.run().await {
            tracing::error!("network error: {}", e);
        }
    });

    // ── Contract engine ───────────────────────────────────────────────────────
    let contracts = Arc::new(ContractEngine::new(chain.clone()));

    // ── Broadcast channel (entries → net gossip) ──────────────────────────────
    // Capacity 256: bursts are fine; slow receivers are dropped.
    let (tx_broadcast, _rx) = tokio::sync::broadcast::channel(256);

    // ── HTTP API ──────────────────────────────────────────────────────────────
    let app_state = api::AppState {
        chain: chain.clone(),
        contracts,
        tx_broadcast,
    };
    api::serve(app_state, cfg.api_port).await?;

    Ok(())
}

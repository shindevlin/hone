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
mod utils;

use std::sync::Arc;
use anyhow::Result;
use tracing::{info, warn};
use btcpc_types::{Block, LedgerEntry, EPOCH_MS};

use chain::Chain;
use config::Config;
use contracts::ContractEngine;
use net::{NetCmd, NetworkEvent};
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

    // ── Networking ────────────────────────────────────────────────────────────
    let (network, net_handle, net_events) = net::Network::new(cfg.clone());
    tokio::spawn(async move {
        if let Err(e) = network.run().await {
            tracing::error!("network error: {}", e);
        }
    });

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

    // Wire: clock sealed events → chain epoch advancement
    {
        let mut sealed_rx = clock.subscribe();
        let chain_ref = chain.clone();
        let node_id_c = cfg.node_id.clone();
        tokio::spawn(async move {
            loop {
                match sealed_rx.recv().await {
                    Ok(sealed) if sealed.sealed => {
                        let seal_hash = sealed.winner
                            .as_ref()
                            .map(|w| w.seal_hash.clone())
                            .unwrap_or_default();
                        let ts = sealed.winner
                            .as_ref()
                            .map(|w| w.timestamp)
                            .unwrap_or_else(now_ms);
                        let entry = LedgerEntry::EpochSeal {
                            node_id: node_id_c.clone(),
                            epoch: sealed.epoch,
                            timestamp: ts,
                            seal_hash,
                            signature: None,
                        };
                        if let Err(e) = chain_ref.apply_entry(&entry) {
                            warn!("clock seal apply failed (epoch {}): {}", sealed.epoch, e);
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("clock sealed_rx lagged by {} events", n);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // Emit seals when this node is running as a clock peer
    if cfg.is_clock {
        let clock_ref = clock.clone();
        let cmd_tx = net_handle.cmd_tx.clone();
        let node_id_c = cfg.node_id.clone();
        tokio::spawn(async move {
            let mut last_sent: u64 = 0;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                let now = now_ms();
                let epoch = now / EPOCH_MS;
                if epoch > last_sent {
                    last_sent = epoch;
                    let seal_hash = {
                        use sha2::Digest;
                        let mut h = sha2::Sha256::new();
                        h.update(epoch.to_le_bytes());
                        h.update(node_id_c.as_bytes());
                        hex::encode(h.finalize())
                    };
                    let seal = serde_json::json!({
                        "epoch_number": epoch,
                        "node_id": node_id_c,
                        "timestamp": now,
                        "seal_hash": seal_hash,
                        "signature": null,
                    });
                    // Self-ingest so we count toward quorum on single-node networks.
                    clock_ref.receive_seal(seal.clone());
                    if let Ok(data) = serde_json::to_vec(&seal) {
                        let _ = cmd_tx.send(NetCmd::Broadcast {
                            topic: "btcpc/seals",
                            data,
                        }).await;
                    }
                }
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

    // ── Broadcast channel (entries → net gossip) ──────────────────────────────
    let (tx_broadcast, _) = tokio::sync::broadcast::channel::<LedgerEntry>(256);

    // Forward newly-accepted entries to gossip peers
    {
        let mut net_rx = tx_broadcast.subscribe();
        let cmd_tx = net_handle.cmd_tx.clone();
        tokio::spawn(async move {
            loop {
                match net_rx.recv().await {
                    Ok(entry) => {
                        if let Ok(data) = serde_json::to_vec(&entry) {
                            let _ = cmd_tx.send(NetCmd::Broadcast {
                                topic: "btcpc/entries",
                                data,
                            }).await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("net gossip tx lagged by {} entries", n);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // Apply incoming network events to chain state
    {
        let chain_ref = chain.clone();
        let clock_ref = clock.clone();
        let mut events = net_events;
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(NetworkEvent::Entry { entry }) => {
                        match tx::entry_from_json(&entry) {
                            Ok(e) => {
                                if let Err(e) = tx::validate_and_apply(&chain_ref, &e, None) {
                                    tracing::debug!("net entry rejected: {}", e);
                                }
                            }
                            Err(e) => tracing::debug!("net entry parse error: {}", e),
                        }
                    }
                    Ok(NetworkEvent::Block { epoch, data }) => {
                        if !chain_ref.store.has_block(epoch) {
                            if let Some(block) = Block::from_bytes(&data) {
                                if let Some(entries) = block.payload.get("ledger_entries")
                                    .and_then(|v| serde_json::from_value::<Vec<LedgerEntry>>(v.clone()).ok())
                                {
                                    chain_ref.apply_block_entries(&entries);
                                }
                                let _ = chain_ref.store.write_block(epoch, &data);
                                let mut cur = chain_ref.current_epoch.write();
                                if epoch as u64 > *cur {
                                    *cur = epoch as u64;
                                }
                            }
                        }
                    }
                    Ok(NetworkEvent::EpochSeal { seal }) => {
                        clock_ref.receive_seal(seal);
                    }
                    Ok(_) => {} // PeerConnected / PeerDisconnected
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("net events lagged by {}", n);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // ── Contract engine ───────────────────────────────────────────────────────
    let contracts = Arc::new(ContractEngine::new(chain.clone()));

    // ── HTTP API ──────────────────────────────────────────────────────────────
    let app_state = api::AppState {
        chain: chain.clone(),
        contracts,
        tx_broadcast,
    };
    api::serve(app_state, cfg.api_port).await?;

    Ok(())
}

use utils::now_ms;

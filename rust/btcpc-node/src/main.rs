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
    BTCPC_MINER               — "true" to enable mining
    BTCPC_CLOCK               — "true" to participate in clock consensus
    BTCPC_GENESIS_FILE        — path to genesis.json
    BTCPC_GENESIS_TIMESTAMP   — Unix ms timestamp for genesis block (MUST match on all nodes)
    BTCPC_LOG_LEVEL           — tracing filter (default: btcpc_node=info)
    BTCPC_BOOTSTRAP_PEERS     — comma-separated multiaddrs for DHT bootstrap
    BTCPC_CHAIN_ID            — "btcpc-1" (mainnet) or "btcpc-satoshi" (testnet)
*/

mod api;
mod chain;
mod clock;
mod config;
mod contracts;
mod discovery;
mod finalize;
mod genesis;
mod inference;
mod inference_daemon;
mod miner;
mod net;
mod sim;
mod store;
mod tx;
mod utils;

use std::sync::Arc;
use anyhow::Result;
use tracing::{info, warn};
use btcpc_types::{Block, LedgerEntry, block_reward_at, era, RECYCLE_ERA, RECYCLE_REWARD_RATE, RECYCLE_REWARD_DENOM, RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, TESTNET_CHAIN_ID};

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

    info!("btcpc-node starting — account={} chain={} data={:?}", cfg.account, cfg.chain_id, cfg.data_dir);

    // Open state database
    let db_path = cfg.data_dir.join("state");
    let store = Store::open(&db_path)?;

    // Network gets a clone of the store for peer-store persistence.
    let (network, net_handle, net_events) = net::Network::new(cfg.clone(), store.clone());

    let chain = Arc::new(Chain::new(store, cfg.node_id.clone(), cfg.chain_id.clone()));

    // Genesis
    genesis::init_genesis(&chain, cfg.genesis_file.as_deref(), cfg.genesis_timestamp)?;

    info!("chain state ready — latest epoch={}", chain.current_epoch());
    tokio::spawn(async move {
        if let Err(e) = network.run().await {
            tracing::error!("network error: {}", e);
        }
    });

    // ── Hive self-announce (best-effort, fire-and-forget) ─────────────────────
    {
        let chain_id = cfg.chain_id.clone();
        let node_id = cfg.node_id.clone();
        tokio::spawn(async move {
            // Small delay so the node is fully up before announcing.
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            discovery::announce_to_hive(&chain_id, &node_id).await;
        });
    }

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
        // Epoch is relative to genesis, not Unix epoch.
        // genesis_ts is guaranteed set (init_genesis would have errored otherwise).
        let genesis_ts = cfg.genesis_timestamp.unwrap_or(0);
        tokio::spawn(async move {
            let mut last_sent: u64 = 0;
            loop {
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                let now = now_ms();
                let elapsed = now.saturating_sub(genesis_ts);
                let epoch = elapsed / btcpc_types::EPOCH_MS;
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

    // ── Inference marketplace daemon ──────────────────────────────────────────
    {
        let chain_ref = chain.clone();
        tokio::spawn(async move {
            inference_daemon::run(chain_ref).await;
        });
    }

    // ── Mining ────────────────────────────────────────────────────────────────
    if cfg.is_miner {
        let chain_ref = chain.clone();
        let account = cfg.account.clone();
        let genesis_ts = cfg.genesis_timestamp.unwrap_or(0);
        tokio::spawn(async move {
            miner::run_miner(chain_ref, account, genesis_ts).await;
        });
    }

    // ── Broadcast channel (entries → net gossip) ──────────────────────────────
    let (tx_broadcast, _) = tokio::sync::broadcast::channel::<api::GossipEntry>(256);

    // Forward newly-accepted entries to gossip peers.
    // Wrap as {"entry": <json>, "sig": <hex_or_null>} so receiving nodes can
    // re-verify signatures for accounts that have registered keys.
    {
        let mut net_rx = tx_broadcast.subscribe();
        let cmd_tx = net_handle.cmd_tx.clone();
        tokio::spawn(async move {
            loop {
                match net_rx.recv().await {
                    Ok((entry, sig)) => {
                        if let Ok(entry_val) = serde_json::to_value(&entry) {
                            let envelope = serde_json::json!({
                                "entry": entry_val,
                                "sig": sig,
                            });
                            if let Ok(data) = serde_json::to_vec(&envelope) {
                                let _ = cmd_tx.send(NetCmd::Broadcast {
                                    topic: "btcpc/entries",
                                    data,
                                }).await;
                            }
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
                        // Unwrap gossip envelope {"entry": ..., "sig": ...}.
                        // Fall back to treating the whole value as the entry (legacy/direct format).
                        let (entry_val, sig) = if let Some(inner) = entry.get("entry") {
                            let sig = entry.get("sig")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                                .map(str::to_owned);
                            (inner.clone(), sig)
                        } else {
                            (entry, None)
                        };
                        match tx::entry_from_json(&entry_val) {
                            Ok(e) => {
                                if let Err(e) = tx::validate_and_apply(&chain_ref, &e, sig.as_deref()) {
                                    tracing::debug!("net entry rejected: {}", e);
                                }
                            }
                            Err(e) => tracing::debug!("net entry parse error: {}", e),
                        }
                    }
                    Ok(NetworkEvent::Block { epoch, data }) => {
                        if !chain_ref.store.has_block(epoch) {
                            if let Some(block) = Block::from_bytes(&data) {
                                let cur = chain_ref.current_epoch();
                                // Reject blocks too far ahead (max +10 epochs).
                                if epoch > cur + 10 {
                                    warn!("gossip block {} too far ahead of epoch {} — ignoring", epoch, cur);
                                } else if let Some(entries) = block.payload.get("ledger_entries")
                                    .and_then(|v| serde_json::from_value::<Vec<LedgerEntry>>(v.clone()).ok())
                                {
                                    // Reject blocks containing privileged self-issued entries.
                                    let has_privileged = entries.iter().any(|e| matches!(
                                        e,
                                        LedgerEntry::GenesisAlloc { .. }
                                        | LedgerEntry::EpochFinalize { .. }
                                    ));
                                    // Reject MineReward without a corresponding Mine entry.
                                    let has_mine_reward = entries.iter().any(|e| matches!(e, LedgerEntry::MineReward { .. }));
                                    let has_mine = entries.iter().any(|e| matches!(e, LedgerEntry::Mine { .. }));
                                    // Reject inflated MineReward — amount must not exceed emission/recycle schedule.
                                    let reward_inflated = entries.iter().any(|e| {
                                        if let LedgerEntry::MineReward { amount, epoch: reward_epoch, .. } = e {
                                            let re = *reward_epoch;
                                            if era(re) >= RECYCLE_ERA {
                                                // Era 5+: capped at RECYCLE_REWARD_RATE/DENOM of fund balance.
                                                let fund = chain_ref.store.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN);
                                                let max = ((fund as u128 * RECYCLE_REWARD_RATE) / RECYCLE_REWARD_DENOM) as u64;
                                                *amount > max
                                            } else {
                                                *amount > block_reward_at(re)
                                            }
                                        } else {
                                            false
                                        }
                                    });
                                    // Reject Transfer entries with invalid nonces (already-used or wrong).
                                    let has_bad_nonce = entries.iter().any(|e| {
                                        if let LedgerEntry::Transfer { from, nonce, .. } = e {
                                            let expected = chain_ref.store.get_account(from)
                                                .ok().flatten()
                                                .and_then(|s| s.get("nonce").and_then(|v| v.as_u64()))
                                                .map(|n| n + 1)
                                                .unwrap_or(1);
                                            *nonce < expected // used or skipped
                                        } else {
                                            false
                                        }
                                    });

                                    // Reject blocks from a different chain (cross-chain replay prevention).
                                    let chain_id_mismatch = block.payload
                                        .get("chain_id")
                                        .and_then(|v| v.as_str())
                                        .map_or(false, |cid| cid != chain_ref.chain_id);

                                    if has_privileged {
                                        warn!("gossip block {} contains privileged entries — rejected", epoch);
                                    } else if chain_id_mismatch {
                                        warn!("gossip block {} chain_id mismatch (expected '{}') — rejected", epoch, chain_ref.chain_id);
                                    } else if has_mine_reward && !has_mine {
                                        warn!("gossip block {} has MineReward without Mine — rejected", epoch);
                                    } else if reward_inflated {
                                        warn!("gossip block {} has inflated MineReward vs emission schedule — rejected", epoch);
                                    } else if has_bad_nonce {
                                        warn!("gossip block {} contains replay or bad-nonce transfers — rejected", epoch);
                                    } else {
                                        chain_ref.apply_block_entries(&entries);
                                        let _ = chain_ref.store.write_block(epoch, &data);
                                        let mut cur = chain_ref.current_epoch.write();
                                        if epoch > *cur {
                                            *cur = epoch;
                                        }
                                    }
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

    // ── Testnet sim daemon ────────────────────────────────────────────────────
    if cfg.chain_id == TESTNET_CHAIN_ID {
        let chain_ref = chain.clone();
        tokio::spawn(async move {
            sim::run(chain_ref).await;
        });
    }

    // ── Inference verifier ────────────────────────────────────────────────────
    {
        let chain_ref = chain.clone();
        let account   = cfg.account.clone();
        let cmd_tx    = net_handle.cmd_tx.clone();
        tokio::spawn(async move {
            run_inference_verifier(chain_ref, account, cmd_tx).await;
        });
    }

    // ── HTTP API ──────────────────────────────────────────────────────────────
    let app_state = api::AppState {
        chain: chain.clone(),
        contracts,
        tx_broadcast,
        faucet_claims: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
    };
    api::serve(app_state, cfg.api_port).await?;

    Ok(())
}

use utils::now_ms;

// ── Inference verifier ────────────────────────────────────────────────────────
//
// Periodically scans for completed inference jobs from OTHER nodes, re-runs
// the same prompt via Ollama using the same model, and submits a verdict.
// Only verifies jobs whose model matches the locally configured BTCPC_MODEL.

async fn run_inference_verifier(
    chain:   Arc<Chain>,
    account: String,
    cmd_tx:  tokio::sync::mpsc::Sender<NetCmd>,
) {
    use std::time::Duration;
    use sha2::{Digest, Sha256};

    let ollama_url = std::env::var("OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned());
    let local_model = std::env::var("BTCPC_MODEL")
        .unwrap_or_else(|_| "qwen2.5:0.5b".to_owned());

    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;

        let epoch = chain.current_epoch();
        let jobs = chain.store.state_scan_prefix("infer_job:");

        for (_, val) in jobs {
            let job: serde_json::Value = match serde_json::from_slice(&val) {
                Ok(v) => v, Err(_) => continue,
            };

            let status_  = job["status"].as_str().unwrap_or("");
            let worker   = job["winner"].as_str().unwrap_or("");
            let model    = job["model"].as_str().unwrap_or("");
            let job_id   = match job["job_id"].as_str() { Some(s) => s.to_owned(), None => continue };
            let result_hash = match job["result_hash"].as_str() { Some(s) => s.to_owned(), None => continue };
            let input_hash  = job["input_hash"].as_str().unwrap_or("hello").to_owned();

            if worker == account || status_ != "complete" { continue; }
            if !model.is_empty() && model != local_model { continue; }

            let verdict_key = format!("infer_verdict:{}:{}", job_id, account);
            if chain.store.state_get(&verdict_key).is_some() { continue; }

            // Re-run inference via Ollama.
            let client = reqwest::Client::new();
            let resp = client
                .post(format!("{}/api/generate", ollama_url))
                .timeout(Duration::from_secs(30))
                .json(&serde_json::json!({
                    "model":  local_model,
                    "prompt": input_hash,
                    "stream": false,
                    "options": { "num_predict": 256, "temperature": 0.0 },
                }))
                .send()
                .await;

            let verdict = match resp {
                Ok(r) if r.status().is_success() => {
                    match r.json::<serde_json::Value>().await {
                        Ok(body) => {
                            let text = body["response"].as_str().unwrap_or("");
                            let our_hash = hex::encode(Sha256::digest(text.as_bytes()));
                            if our_hash == result_hash { "approved" } else { "disputed" }
                        }
                        Err(_) => continue,
                    }
                }
                _ => continue,
            };

            let entry = LedgerEntry::InferenceJobVerify {
                job_id:    job_id.clone(),
                verifier:  account.clone(),
                verdict:   verdict.to_owned(),
                reason:    None,
                epoch,
                signed_by: account.clone(),
            };
            if let Err(e) = chain.apply_entry(&entry) {
                warn!("verifier: apply failed for job {}: {}", job_id, e);
                continue;
            }
            let envelope = serde_json::json!({ "entry": entry });
            if let Ok(data) = serde_json::to_vec(&envelope) {
                let _ = cmd_tx.send(NetCmd::Broadcast {
                    topic: "btcpc/entries", data,
                }).await;
            }
            let _ = chain.store.state_set(&verdict_key, verdict.as_bytes());
            info!("verifier: job {} → {}", job_id, verdict);

            break; // one verdict per cycle
        }
    }
}

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
use btcpc_types::{
    LedgerEntry, block_reward_at, era, RECYCLE_ERA, RECYCLE_REWARD_RATE, RECYCLE_REWARD_DENOM,
    RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, TESTNET_CHAIN_ID,
    EPOCH_POOL_INFERENCE_BPS, EPOCH_POOL_STORAGE_BPS, EPOCH_POOL_SENSOR_BPS,
    EPOCH_POOL_VERIFY_BPS, EPOCH_POOL_RECYCLE_BPS, inference_score,
};

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

    // Wire: clock sealed events → chain epoch advancement + MineReward emission
    {
        let mut sealed_rx = clock.subscribe();
        let chain_ref = chain.clone();
        let node_id_c = cfg.node_id.clone();
        let cmd_tx_for_seal = net_handle.cmd_tx.clone();
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

                        // Comprehensive epoch reward distribution across all work types.
                        let sealed_epoch = sealed.epoch;
                        emit_epoch_rewards(sealed_epoch, &chain_ref, &cmd_tx_for_seal).await;
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
        let cmd_for_miner = net_handle.cmd_tx.clone();
        tokio::spawn(async move {
            miner::run_miner(chain_ref, account, genesis_ts, cmd_for_miner).await;
        });
    }

    // ── Broadcast channel (entries → net gossip) ──────────────────────────────
    let (tx_broadcast, _) = tokio::sync::broadcast::channel::<api::GossipEntry>(256);

    // Forward newly-accepted entries to gossip peers.
    // Wrap as {"entry": <json>, "sig": <hex_or_null>} so receiving nodes can
    // re-verify signatures for accounts that have registered keys.
    // For InferenceJobPost and InferenceJobComplete, also carry the actual
    // input_text / output_text so remote verifiers can assess quality.
    {
        let mut net_rx = tx_broadcast.subscribe();
        let cmd_tx = net_handle.cmd_tx.clone();
        let chain_ref = chain.clone();
        tokio::spawn(async move {
            loop {
                match net_rx.recv().await {
                    Ok((entry, sig)) => {
                        if let Ok(entry_val) = serde_json::to_value(&entry) {
                            let mut envelope = serde_json::json!({
                                "entry": entry_val,
                                "sig": sig,
                            });
                            // Attach plaintext so remote verifiers can judge quality.
                            match &entry {
                                LedgerEntry::InferenceJobPost { job_id, .. } => {
                                    if let Some(t) = chain_ref.store.state_get(
                                        &format!("infer_input:{}", job_id)
                                    ).and_then(|b| String::from_utf8(b).ok()) {
                                        envelope["input_text"] = serde_json::Value::String(t);
                                    }
                                }
                                LedgerEntry::InferenceJobComplete { job_id, .. } => {
                                    if let Some(t) = chain_ref.store.state_get(
                                        &format!("infer_output:{}", job_id)
                                    ).and_then(|b| String::from_utf8(b).ok()) {
                                        envelope["output_text"] = serde_json::Value::String(t);
                                    }
                                }
                                _ => {}
                            }
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
                        // Extract and store any plaintext attached to the gossip envelope
                        // so the local verifier can access it later.
                        if let Some(job_id) = entry.get("entry")
                            .and_then(|e| e.get("InferenceJobPost"))
                            .and_then(|v| v.get("job_id"))
                            .and_then(|v| v.as_str())
                        {
                            if let Some(t) = entry.get("input_text").and_then(|v| v.as_str()) {
                                let _ = chain_ref.store.state_set(
                                    &format!("infer_input:{}", job_id), t.as_bytes());
                            }
                        }
                        if let Some(job_id) = entry.get("entry")
                            .and_then(|e| e.get("InferenceJobComplete"))
                            .and_then(|v| v.get("job_id"))
                            .and_then(|v| v.as_str())
                        {
                            if let Some(t) = entry.get("output_text").and_then(|v| v.as_str()) {
                                let _ = chain_ref.store.state_set(
                                    &format!("infer_output:{}", job_id), t.as_bytes());
                            }
                        }

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
                    Ok(NetworkEvent::Block { .. }) => {
                        // Miners no longer produce blocks — block gossip is ignored.
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
            use inference::JobState;
            let job: JobState = match serde_json::from_slice(&val) {
                Ok(v) => v, Err(_) => continue,
            };

            // Only verify completed jobs from other nodes.
            if job.winner.as_deref() == Some(account.as_str()) { continue; }
            if job.status != inference::JobStatus::Completed { continue; }
            if job.model != local_model { continue; }

            let job_id = &job.job_id;
            let verdict_key = format!("infer_verdict:{}:{}", job_id, account);
            if chain.store.state_get(&verdict_key).is_some() { continue; }

            // Need both the original request and the submitted output to judge quality.
            let input_text = match chain.store.state_get(&format!("infer_input:{}", job_id))
                .and_then(|b| String::from_utf8(b).ok())
            {
                Some(t) => t,
                None => continue, // can't assess without the actual input
            };
            let output_text = match chain.store.state_get(&format!("infer_output:{}", job_id))
                .and_then(|b| String::from_utf8(b).ok())
            {
                Some(t) => t,
                None => continue, // can't assess without the actual output
            };

            // Ask the local model to evaluate whether the work was done.
            let meta_prompt = format!(
                "Task: {}\nResponse: {}\n\n\
                Did the AI assistant adequately complete the task?\n\
                Reply with exactly one word: APPROVED, REJECTED, or REVIEW.",
                input_text.chars().take(512).collect::<String>(),
                output_text.chars().take(512).collect::<String>(),
            );

            let client = reqwest::Client::new();
            let resp = client
                .post(format!("{}/api/generate", ollama_url))
                .timeout(Duration::from_secs(30))
                .json(&serde_json::json!({
                    "model":  local_model,
                    "prompt": meta_prompt,
                    "stream": false,
                    "options": { "num_predict": 20, "temperature": 0.0 },
                }))
                .send()
                .await;

            let verdict = match resp {
                Ok(r) if r.status().is_success() => {
                    match r.json::<serde_json::Value>().await {
                        Ok(body) => {
                            let text = body["response"].as_str().unwrap_or("").to_uppercase();
                            if text.contains("APPROVED") {
                                "approved"
                            } else if text.contains("REJECTED") || text.contains("REJECT") {
                                "rejected"
                            } else {
                                "review_required"
                            }
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

// ── Epoch reward distribution ─────────────────────────────────────────────────

/// Distribute the epoch's reward pool across inference, storage, sensor, and verify work.
async fn emit_epoch_rewards(
    epoch: u64,
    chain: &Arc<Chain>,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
) {
    let total_pool = if era(epoch) >= RECYCLE_ERA {
        let fund = chain.store.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN);
        ((fund as u128 * RECYCLE_REWARD_RATE) / RECYCLE_REWARD_DENOM) as u64
    } else {
        block_reward_at(epoch)
    };
    if total_pool == 0 { return; }

    let inference_pool = total_pool * EPOCH_POOL_INFERENCE_BPS / 10_000;
    let storage_pool   = total_pool * EPOCH_POOL_STORAGE_BPS   / 10_000;
    let sensor_pool    = total_pool * EPOCH_POOL_SENSOR_BPS    / 10_000;
    let verify_pool    = total_pool * EPOCH_POOL_VERIFY_BPS    / 10_000;
    let recycle_amt    = total_pool * EPOCH_POOL_RECYCLE_BPS   / 10_000;

    // ── Inference pool ────────────────────────────────────────────────────────
    let mines: Vec<(String, u64)> = chain.store.state_scan_prefix(&format!("mine:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let miner    = j["miner"].as_str()?.to_owned();
            let out_toks = j["output_tokens"].as_u64().unwrap_or(0);
            let hw_tier  = j["hw_tier"].as_u64().unwrap_or(0) as u8;
            let model    = j["model"].as_str().unwrap_or("");
            Some((miner, inference_score(out_toks, hw_tier, model)))
        }).collect();

    distribute_rewards_desktop(epoch, &mines, inference_pool, chain, cmd_tx, |miner, amount, ep| {
        LedgerEntry::MineReward { miner, amount, epoch: ep }
    }).await;

    // ── Storage pool ──────────────────────────────────────────────────────────
    let storage_nodes: Vec<(String, u64)> = chain.store.state_scan_prefix(&format!("storage_beat:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let node_id = j["node_id"].as_str()?.to_owned();
            let bytes   = j["bytes_proven"].as_u64().unwrap_or(0);
            Some((node_id, bytes))
        }).collect();

    distribute_rewards_desktop(epoch, &storage_nodes, storage_pool, chain, cmd_tx, |node_id, amount, ep| {
        LedgerEntry::ClockReward { node_id, amount, epoch: ep }
    }).await;

    // ── Sensor pool ───────────────────────────────────────────────────────────
    let sensor_nodes: Vec<(String, u64)> = chain.store.state_scan_prefix(&format!("sensor_commit:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let owner = j["owner"].as_str()?.to_owned();
            let count = j["reading_count"].as_u64().unwrap_or(0);
            Some((owner, count))
        }).collect();

    distribute_rewards_desktop(epoch, &sensor_nodes, sensor_pool, chain, cmd_tx, |node_id, amount, ep| {
        LedgerEntry::ClockReward { node_id, amount, epoch: ep }
    }).await;

    // ── Verify pool ───────────────────────────────────────────────────────────
    let verifiers: Vec<(String, u64)> = chain.store.state_scan_prefix(&format!("infer_verify:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let verifier = j["verifier"].as_str()?.to_owned();
            let count    = j["count"].as_u64().unwrap_or(0);
            Some((verifier, count))
        }).collect();

    distribute_rewards_desktop(epoch, &verifiers, verify_pool, chain, cmd_tx, |node_id, amount, ep| {
        LedgerEntry::ClockReward { node_id, amount, epoch: ep }
    }).await;

    // ── Recycle fund ──────────────────────────────────────────────────────────
    if recycle_amt > 0 {
        if let Err(e) = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_amt) {
            warn!("clock: recycle credit failed epoch {}: {}", epoch, e);
        } else {
            info!("clock: epoch {} → recycle fund +{} dreams", epoch, recycle_amt);
        }
    }

    info!("clock: epoch {} rewards — pool={} inference={} storage={} sensor={} verify={}",
        epoch, total_pool, inference_pool, storage_pool, sensor_pool, verify_pool);
}

async fn distribute_rewards_desktop<F>(
    epoch: u64,
    nodes: &[(String, u64)],
    pool: u64,
    chain: &Arc<Chain>,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
    make_entry: F,
) where
    F: Fn(String, u64, u64) -> LedgerEntry,
{
    if nodes.is_empty() || pool == 0 { return; }
    let total: u64 = nodes.iter().map(|(_, s)| s).sum();
    let n = nodes.len() as u64;
    for (account, score) in nodes {
        let amount = if total == 0 { pool / n }
            else { (pool as u128 * *score as u128 / total as u128) as u64 };
        if amount == 0 { continue; }
        let entry = make_entry(account.clone(), amount, epoch);
        if let Err(e) = chain.apply_entry(&entry) {
            warn!("clock: reward apply failed for {} epoch {}: {}", account, epoch, e);
            continue;
        }
        let envelope = serde_json::json!({"entry": entry});
        if let Ok(data) = serde_json::to_vec(&envelope) {
            let _ = cmd_tx.send(NetCmd::Broadcast {
                topic: "btcpc/entries",
                data,
            }).await;
        }
    }
}

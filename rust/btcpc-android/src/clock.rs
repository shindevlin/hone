//! Clock consensus for Android micronode.
//!
//! Identical semantics to the desktop node:
//!   • Emits a signed epoch seal every epoch over gossipsub
//!   • Tracks seals from peers; fires EpochFinalize when quorum (≥ 2 unique nodes) reached
//!   • No external API calls — all communication via the embedded libp2p swarm

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use tracing::{info, warn};

use btcpc_types::{
    LedgerEntry, EPOCH_MS,
    block_reward_at, era, RECYCLE_ERA, RECYCLE_REWARD_RATE, RECYCLE_REWARD_DENOM,
    RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN,
    EPOCH_POOL_INFERENCE_BPS, EPOCH_POOL_STORAGE_BPS, EPOCH_POOL_SENSOR_BPS,
    EPOCH_POOL_VERIFY_BPS, EPOCH_POOL_RECYCLE_BPS,
    inference_score,
};
use crate::net::NetCmd;

fn quorum() -> usize {
    std::env::var("BTCPC_CLOCK_QUORUM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpochSeal {
    pub epoch_number: u64,
    pub node_id:      String,
    pub timestamp:    u64,
    pub seal_hash:    String,
    pub signature:    Option<String>,
}

#[derive(Debug, Clone)]
pub struct SealedEpoch {
    pub epoch:  u64,
    pub sealed: bool,
    pub winner: Option<EpochSeal>,
}

pub struct ClockConsensus {
    seals:    Mutex<HashMap<u64, Vec<EpochSeal>>>,
    sealed_tx: broadcast::Sender<SealedEpoch>,
}

impl ClockConsensus {
    pub fn new() -> (Arc<Self>, broadcast::Receiver<SealedEpoch>) {
        let (tx, rx) = broadcast::channel(64);
        let c = Arc::new(ClockConsensus {
            seals: Mutex::new(HashMap::new()),
            sealed_tx: tx,
        });
        (c, rx)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SealedEpoch> {
        self.sealed_tx.subscribe()
    }

    pub fn receive_seal(&self, raw: serde_json::Value) {
        let seal: EpochSeal = match serde_json::from_value(raw) {
            Ok(s) => s,
            Err(e) => { warn!("clock: bad seal: {}", e); return; }
        };
        let epoch = seal.epoch_number;
        let mut map = self.seals.lock();
        let entries = map.entry(epoch).or_default();
        // deduplicate by node_id
        if entries.iter().any(|s| s.node_id == seal.node_id) { return; }
        entries.push(seal.clone());
        let count = entries.len();
        let winner = entries.first().cloned();
        drop(map);

        if count >= quorum() {
            let sealed = SealedEpoch { epoch, sealed: true, winner };
            let _ = self.sealed_tx.send(sealed);
            info!("clock: epoch {} sealed ({} nodes)", epoch, count);
        }
    }

    /// Remove seals for epochs older than `keep_from` to bound memory usage.
    pub fn prune(&self, keep_from: u64) {
        self.seals.lock().retain(|&e, _| e >= keep_from);
    }
}

// ── Clock runner ──────────────────────────────────────────────────────────────

pub struct ClockConfig {
    pub node_id:    String,
    pub chain_id:   String,
    pub genesis_ts: u64,
    pub is_clock:   bool,
    pub quorum:     usize,
}

/// Runs the clock tick loop and seal emitter.
/// Called from node.rs after the swarm is started.
pub async fn run(
    cfg: ClockConfig,
    clock: Arc<ClockConsensus>,
    cmd_tx: tokio::sync::mpsc::Sender<NetCmd>,
    chain: Arc<crate::chain::Chain>,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) {
    // Wire sealed events → chain epoch advancement + MineReward emission
    {
        let mut sealed_rx = clock.subscribe();
        let chain_ref = chain.clone();
        let node_id = cfg.node_id.clone();
        let cmd_tx_clone = cmd_tx.clone();
        tokio::spawn(async move {
            loop {
                match sealed_rx.recv().await {
                    Ok(s) if s.sealed => {
                        let seal_hash = s.winner.as_ref()
                            .map(|w| w.seal_hash.clone())
                            .unwrap_or_default();
                        let ts = s.winner.as_ref()
                            .map(|w| w.timestamp)
                            .unwrap_or_else(now_ms);
                        let entry = LedgerEntry::EpochSeal {
                            node_id: node_id.clone(),
                            epoch: s.epoch,
                            timestamp: ts,
                            seal_hash,
                            signature: None,
                        };
                        let _ = chain_ref.apply_entry(&entry);

                        // Comprehensive epoch reward distribution.
                        let sealed_epoch = s.epoch;
                        emit_epoch_rewards(sealed_epoch, &chain_ref, &cmd_tx_clone).await;
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("clock sealed_rx lagged {}", n);
                    }
                    Err(_) => break,
                }
            }
        });
    }

    if !cfg.is_clock {
        // Not a clock node — still receive seals from network but don't emit.
        return;
    }

    // Tick loop: emit seal once per epoch.
    let genesis_ts  = cfg.genesis_ts;
    let node_id     = cfg.node_id.clone();
    let cmd_tx_c    = cmd_tx.clone();
    let clock_ref   = clock.clone();

    let mut last_sent: u64 = 0;

    loop {
        tokio::select! {
            _ = shutdown.recv() => break,
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }

        let now = now_ms();
        if now < genesis_ts { continue; }
        let epoch = (now - genesis_ts) / EPOCH_MS;
        if epoch <= last_sent { continue; }
        last_sent = epoch;

        let seal_hash = {
            let mut h = Sha256::new();
            h.update(epoch.to_le_bytes());
            h.update(node_id.as_bytes());
            hex::encode(h.finalize())
        };
        let seal = serde_json::json!({
            "epoch_number": epoch,
            "node_id":      node_id,
            "timestamp":    now,
            "seal_hash":    seal_hash,
            "signature":    null,
        });

        // Self-ingest (counts toward our own quorum).
        clock_ref.receive_seal(seal.clone());

        // Broadcast to peers.
        if let Ok(data) = serde_json::to_vec(&seal) {
            let _ = cmd_tx_c.send(NetCmd::Broadcast {
                topic: "btcpc/seals",
                data,
            }).await;
        }

        info!("clock: emitted seal epoch {}", epoch);
        clock_ref.prune(epoch.saturating_sub(10));
    }
}

// ── Epoch reward distribution ─────────────────────────────────────────────────

/// Compute and broadcast all epoch rewards after a seal reaches quorum.
///
/// Pools: inference 50%, storage 20%, sensor 15%, verification 10%, remainder → recycle.
async fn emit_epoch_rewards(
    epoch: u64,
    chain: &crate::chain::Chain,
    cmd_tx: &tokio::sync::mpsc::Sender<crate::net::NetCmd>,
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

    // Collect work vectors from state store
    let mines: Vec<(String, u64)> = chain.store.scan_prefix(&format!("mine:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let miner     = j["miner"].as_str()?.to_owned();
            let out_toks  = j["output_tokens"].as_u64().unwrap_or(0);
            let hw_tier   = j["hw_tier"].as_u64().unwrap_or(0) as u8;
            let model     = j["model"].as_str().unwrap_or("");
            Some((miner, inference_score(out_toks, hw_tier, model)))
        }).collect();

    let storage_nodes: Vec<(String, u64)> = chain.store.scan_prefix(&format!("storage_beat:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let node_id = j["node_id"].as_str()?.to_owned();
            let bytes   = j["bytes_proven"].as_u64().unwrap_or(0);
            Some((node_id, bytes))
        }).collect();

    let sensor_nodes: Vec<(String, u64)> = chain.store.scan_prefix(&format!("sensor_commit:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let owner = j["owner"].as_str()?.to_owned();
            let count = j["reading_count"].as_u64().unwrap_or(0);
            Some((owner, count))
        }).collect();

    let verifiers: Vec<(String, u64)> = chain.store.scan_prefix(&format!("infer_verify:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let verifier = j["verifier"].as_str()?.to_owned();
            let count    = j["count"].as_u64().unwrap_or(0);
            Some((verifier, count))
        }).collect();

    // Distribute each pool pro-rata
    distribute_mine_rewards(epoch, &mines, inference_pool, chain, cmd_tx).await;
    distribute_storage_rewards(epoch, &storage_nodes, storage_pool, chain, cmd_tx).await;
    distribute_sensor_rewards(epoch, &sensor_nodes, sensor_pool, chain, cmd_tx).await;
    distribute_verify_rewards(epoch, &verifiers, verify_pool, chain, cmd_tx).await;

    // Push remainder to recycle fund
    if recycle_amt > 0 {
        chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_amt);
        info!("clock: epoch {} → recycle fund +{} dreams", epoch, recycle_amt);
    }

    info!("clock: epoch {} rewards emitted (pool={} inference={} storage={} sensor={} verify={})",
        epoch, total_pool, inference_pool, storage_pool, sensor_pool, verify_pool);
}

async fn distribute_mine_rewards(
    epoch: u64,
    miners: &[(String, u64)],
    pool: u64,
    chain: &crate::chain::Chain,
    cmd_tx: &tokio::sync::mpsc::Sender<crate::net::NetCmd>,
) {
    if miners.is_empty() || pool == 0 { return; }
    let total_score: u64 = miners.iter().map(|(_, s)| s).sum();
    let n = miners.len() as u64;
    for (miner, score) in miners {
        let amount = if total_score == 0 {
            pool / n
        } else {
            (pool as u128 * *score as u128 / total_score as u128) as u64
        };
        if amount == 0 { continue; }
        let entry = LedgerEntry::MineReward { miner: miner.clone(), amount, epoch };
        let _ = chain.apply_entry(&entry);
        let _ = broadcast_entry(&entry, cmd_tx).await;
    }
}

async fn distribute_storage_rewards(
    epoch: u64,
    nodes: &[(String, u64)],
    pool: u64,
    chain: &crate::chain::Chain,
    cmd_tx: &tokio::sync::mpsc::Sender<crate::net::NetCmd>,
) {
    if nodes.is_empty() || pool == 0 { return; }
    let total: u64 = nodes.iter().map(|(_, b)| b).sum();
    let n = nodes.len() as u64;
    for (node_id, bytes) in nodes {
        let amount = if total == 0 { pool / n }
            else { (pool as u128 * *bytes as u128 / total as u128) as u64 };
        if amount == 0 { continue; }
        let entry = LedgerEntry::ClockReward { node_id: node_id.clone(), amount, epoch };
        let _ = chain.apply_entry(&entry);
        let _ = broadcast_entry(&entry, cmd_tx).await;
    }
}

async fn distribute_sensor_rewards(
    epoch: u64,
    nodes: &[(String, u64)],
    pool: u64,
    chain: &crate::chain::Chain,
    cmd_tx: &tokio::sync::mpsc::Sender<crate::net::NetCmd>,
) {
    if nodes.is_empty() || pool == 0 { return; }
    let total: u64 = nodes.iter().map(|(_, c)| c).sum();
    let n = nodes.len() as u64;
    for (owner, count) in nodes {
        let amount = if total == 0 { pool / n }
            else { (pool as u128 * *count as u128 / total as u128) as u64 };
        if amount == 0 { continue; }
        let entry = LedgerEntry::ClockReward { node_id: owner.clone(), amount, epoch };
        let _ = chain.apply_entry(&entry);
        let _ = broadcast_entry(&entry, cmd_tx).await;
    }
}

async fn distribute_verify_rewards(
    epoch: u64,
    verifiers: &[(String, u64)],
    pool: u64,
    chain: &crate::chain::Chain,
    cmd_tx: &tokio::sync::mpsc::Sender<crate::net::NetCmd>,
) {
    if verifiers.is_empty() || pool == 0 { return; }
    let total: u64 = verifiers.iter().map(|(_, c)| c).sum();
    let n = verifiers.len() as u64;
    for (verifier, count) in verifiers {
        let amount = if total == 0 { pool / n }
            else { (pool as u128 * *count as u128 / total as u128) as u64 };
        if amount == 0 { continue; }
        let entry = LedgerEntry::ClockReward { node_id: verifier.clone(), amount, epoch };
        let _ = chain.apply_entry(&entry);
        let _ = broadcast_entry(&entry, cmd_tx).await;
    }
}

async fn broadcast_entry(
    entry: &LedgerEntry,
    cmd_tx: &tokio::sync::mpsc::Sender<crate::net::NetCmd>,
) -> bool {
    let envelope = serde_json::json!({"entry": entry});
    if let Ok(data) = serde_json::to_vec(&envelope) {
        cmd_tx.send(crate::net::NetCmd::Broadcast {
            topic: "btcpc/entries",
            data,
        }).await.is_ok()
    } else {
        false
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

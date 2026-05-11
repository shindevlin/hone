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
    RECYCLE_FUND_ACCOUNT, TESTNET_FUND_ACCOUNT, NATIVE_TOKEN,
    inference_score, clock_reward_at, testnet_reward_at,
    sensor_score, stake_weight, mempool_relay_score,
    ACTIVITY_RATIO_DENOM, MIN_ACTIVITY_RATIO_NUM,
    CALIBRATION_INFERENCE, CALIBRATION_STORAGE, CALIBRATION_SENSOR,
    CALIBRATION_VERIFIER, CALIBRATION_SERVICE, CALIBRATION_MEMPOOL,
    CRITICAL_MASS_INFERENCE, CRITICAL_MASS_STORAGE, CRITICAL_MASS_SENSOR,
    CRITICAL_MASS_VERIFIER, CRITICAL_MASS_SERVICE, CRITICAL_MASS_MEMPOOL,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardProposal {
    pub epoch: u64,
    pub node_id: String,
    pub rewards_hash: String,
}

#[derive(Debug, Clone)]
pub struct FinalizedEpoch {
    pub epoch: u64,
    pub rewards_hash: String,
    pub quorum: usize,
}

struct RewardState {
    proposals: Vec<RewardProposal>,
    finalized: bool,
}

pub struct ClockConsensus {
    seals:         Mutex<HashMap<u64, Vec<EpochSeal>>>,
    reward_states: Mutex<HashMap<u64, RewardState>>,
    sealed_tx:    broadcast::Sender<SealedEpoch>,
    finalized_tx: broadcast::Sender<FinalizedEpoch>,
}

impl ClockConsensus {
    pub fn new() -> (Arc<Self>, broadcast::Receiver<SealedEpoch>) {
        let (sealed_tx, rx) = broadcast::channel(64);
        let (finalized_tx, _) = broadcast::channel(64);
        let c = Arc::new(ClockConsensus {
            seals: Mutex::new(HashMap::new()),
            reward_states: Mutex::new(HashMap::new()),
            sealed_tx,
            finalized_tx,
        });
        (c, rx)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SealedEpoch> {
        self.sealed_tx.subscribe()
    }

    pub fn subscribe_finalized(&self) -> broadcast::Receiver<FinalizedEpoch> {
        self.finalized_tx.subscribe()
    }

    pub fn receive_reward_proposal(&self, proposal: RewardProposal) {
        let epoch = proposal.epoch;
        let finalized_event = {
            let mut states = self.reward_states.lock();
            let state = states.entry(epoch).or_insert_with(|| RewardState {
                proposals: Vec::new(),
                finalized: false,
            });
            if state.finalized { return; }
            if !state.proposals.iter().any(|p| p.node_id == proposal.node_id) {
                state.proposals.push(proposal);
            }
            let total = state.proposals.len();
            let quorum_needed = ((total as f64 * 0.51_f64).ceil() as usize).max(quorum());
            let mut hash_counts: HashMap<String, usize> = HashMap::new();
            for p in &state.proposals {
                *hash_counts.entry(p.rewards_hash.clone()).or_default() += 1;
            }
            if let Some((best_hash, &count)) = hash_counts.iter().max_by_key(|(_, c)| *c) {
                if count >= quorum_needed {
                    state.finalized = true;
                    Some(FinalizedEpoch { epoch, rewards_hash: best_hash.clone(), quorum: count })
                } else { None }
            } else { None }
        };
        if let Some(event) = finalized_event {
            info!("clock: epoch {} reward consensus quorum={}", event.epoch, event.quorum);
            let _ = self.finalized_tx.send(event);
        }
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

fn compute_rewards_hash(epoch: u64, store: &crate::store::Store) -> String {
    let mut hasher = Sha256::new();
    let prefixes = [
        format!("mine:{}:", epoch),
        format!("storage_beat:{}:", epoch),
        format!("sensor_commit:{}:", epoch),
        format!("infer_verify:{}:", epoch),
        format!("service_beat:{}:", epoch),
        format!("mempool_beat:{}:", epoch),
    ];
    for prefix in &prefixes {
        let mut entries = store.scan_prefix(prefix);
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (k, v) in entries {
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            hasher.update(&v);
            hasher.update(b"\n");
        }
    }
    format!("{:x}", hasher.finalize())
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
    // Wire sealed events → chain epoch advancement + reward emission + consensus proposal
    {
        let mut sealed_rx = clock.subscribe();
        let chain_ref = chain.clone();
        let clock_ref2 = clock.clone();
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
                            node_version: None,
                        };
                        let _ = chain_ref.apply_entry(&entry);

                        // Comprehensive epoch reward distribution.
                        let sealed_epoch = s.epoch;
                        emit_epoch_rewards(sealed_epoch, &chain_ref, &cmd_tx_clone).await;

                        // Consensus: hash reward inputs and broadcast proposal.
                        let rewards_hash = compute_rewards_hash(sealed_epoch, &chain_ref.store);
                        let proposal = RewardProposal {
                            epoch: sealed_epoch,
                            node_id: node_id.clone(),
                            rewards_hash: rewards_hash.clone(),
                        };
                        clock_ref2.receive_reward_proposal(proposal);
                        if let Ok(data) = serde_json::to_vec(&serde_json::json!({
                            "epoch": sealed_epoch,
                            "node_id": node_id,
                            "rewards_hash": rewards_hash,
                        })) {
                            let _ = cmd_tx_clone.send(NetCmd::Broadcast {
                                topic: "btcpc/consensus",
                                data,
                            }).await;
                        }
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

    // Wire finalized events → emit EpochFinalize entry
    {
        let mut finalized_rx = clock.subscribe_finalized();
        let chain_ref = chain.clone();
        let node_id = cfg.node_id.clone();
        let cmd_tx_fin = cmd_tx.clone();
        tokio::spawn(async move {
            loop {
                match finalized_rx.recv().await {
                    Ok(fin) => {
                        let entry = LedgerEntry::EpochFinalize {
                            epoch: fin.epoch,
                            node_id: node_id.clone(),
                            rewards_hash: fin.rewards_hash,
                            quorum: fin.quorum as u64,
                            sealed_by: vec![],
                            state_root: String::new(),
                            timestamp: now_ms(),
                        };
                        let _ = chain_ref.apply_entry(&entry);
                        let envelope = serde_json::json!({"entry": entry});
                        if let Ok(data) = serde_json::to_vec(&envelope) {
                            let _ = cmd_tx_fin.send(NetCmd::Broadcast {
                                topic: "btcpc/entries",
                                data,
                            }).await;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("finalized_rx lagged {}", n);
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

/// Distribute epoch rewards: mandatory reserve split → Layer D base → Layer B activity pools.
async fn emit_epoch_rewards(
    epoch: u64,
    chain: &crate::chain::Chain,
    cmd_tx: &tokio::sync::mpsc::Sender<crate::net::NetCmd>,
) {
    let raw_pool = if era(epoch) >= RECYCLE_ERA {
        let fund = chain.store.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN);
        ((fund as u128 * RECYCLE_REWARD_RATE) / RECYCLE_REWARD_DENOM) as u64
    } else {
        block_reward_at(epoch)
    };
    if raw_pool == 0 { return; }

    // ── Mandatory 2% reserve split (always fires) ─────────────────────────────
    let reserve_total  = (raw_pool as u128 * 2 / 100) as u64;
    let testnet_top_up = (raw_pool as u128 / 200) as u64;            // 0.5%
    let recycle_split  = reserve_total.saturating_sub(testnet_top_up); // 1.5%
    let activity_pool  = raw_pool.saturating_sub(reserve_total);

    if recycle_split > 0 { chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_split); }
    if testnet_top_up > 0 { chain.store.credit(TESTNET_FUND_ACCOUNT, NATIVE_TOKEN, testnet_top_up); }

    // ── Layer D: era-scaled clock reward ──────────────────────────────────────
    let clock_reward = clock_reward_at(epoch);
    let clock_sealers: Vec<String> = chain.store.scan_prefix(&format!("seal:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            serde_json::from_slice::<serde_json::Value>(&v).ok()
                .and_then(|j| j["node_id"].as_str().map(str::to_owned))
        }).collect();

    for node_id in &clock_sealers {
        if clock_reward == 0 { break; }
        let entry = LedgerEntry::ClockReward { node_id: node_id.clone(), amount: clock_reward, epoch };
        let _ = chain.apply_entry(&entry);
        broadcast_entry(&entry, cmd_tx).await;
    }

    // ── Layer D: testnet operator rewards (from testnet fund) ─────────────────
    let testnet_ops: Vec<(String, String)> = chain.store.scan_prefix("testnet_op:")
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            Some((j["mainnet_account"].as_str()?.to_owned(), j["testnet_node_id"].as_str()?.to_owned()))
        }).collect();

    if !testnet_ops.is_empty() {
        let per_op = testnet_reward_at(epoch);
        let fund   = chain.store.get_balance(TESTNET_FUND_ACCOUNT, NATIVE_TOKEN);
        let needed = per_op.saturating_mul(testnet_ops.len() as u64);
        let actual = if needed > 0 { fund.min(needed) / testnet_ops.len() as u64 } else { 0 };
        for (mainnet_account, testnet_node_id) in &testnet_ops {
            if actual == 0 { break; }
            let entry = LedgerEntry::TestnetReward {
                mainnet_account: mainnet_account.clone(),
                testnet_node_id: testnet_node_id.clone(),
                amount: actual, epoch,
            };
            let _ = chain.apply_entry(&entry);
            broadcast_entry(&entry, cmd_tx).await;
        }
    }

    if activity_pool == 0 { return; }

    // ── Layer B: collect work contributors ────────────────────────────────────

    // Inference: score × stake_weight multiplier
    let mines: Vec<(String, u64)> = chain.store.scan_prefix(&format!("mine:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let out_toks = j["output_tokens"].as_u64().unwrap_or(0);
            let hw_tier  = j["hw_tier"].as_u64().unwrap_or(0) as u8;
            let model    = j["model"].as_str().unwrap_or("");
            let miner    = j["miner"].as_str()?.to_owned();
            let stake    = chain.store.get_stake(&miner);
            let sw       = stake_weight(stake);
            Some((miner, inference_score(out_toks, hw_tier, model).saturating_mul(sw)))
        }).collect();

    let storage_nodes: Vec<(String, u64)> = chain.store.scan_prefix(&format!("storage_beat:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let bytes   = j["bytes_proven"].as_u64().unwrap_or(0);
            let queries = j["query_count"].as_u64().unwrap_or(0);
            let bonus   = (bytes as u128 * queries.min(100) as u128 / 100) as u64;
            Some((j["node_id"].as_str()?.to_owned(), bytes.saturating_add(bonus)))
        }).collect();

    // Sensors: type-aware sensor_score() per sensor_id, grouped by owner
    let sensor_nodes: Vec<(String, u64)> = {
        let mut by_owner: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
        for (_, v) in chain.store.scan_prefix(&format!("sensor_commit:{}:", epoch)) {
            let Ok(j) = serde_json::from_slice::<serde_json::Value>(&v) else { continue };
            let Some(owner) = j["owner"].as_str() else { continue };
            let reading_count = j["reading_count"].as_u64().unwrap_or(0);
            let stype         = j["sensor_type"].as_str().unwrap_or("continuous");
            *by_owner.entry(owner.to_owned()).or_default() += sensor_score(reading_count, stype);
        }
        by_owner.into_iter().collect()
    };

    let verifiers: Vec<(String, u64)> = chain.store.scan_prefix(&format!("infer_verify:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let score = j["value_score_total"].as_u64().unwrap_or(0);
            if score == 0 { return None; }
            Some((j["verifier"].as_str()?.to_owned(), score))
        }).collect();

    let service_nodes: Vec<(String, u64)> = chain.store.scan_prefix(&format!("service_beat:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            Some((j["node_id"].as_str()?.to_owned(), j["container_hours"].as_u64().unwrap_or(0)))
        }).collect();

    let mempool_ops: Vec<(String, u64)> = chain.store.scan_prefix(&format!("mempool_beat:{}:", epoch))
        .into_iter()
        .filter_map(|(_, v)| {
            let j = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
            let latency = j["latency_ms"].as_u64().unwrap_or(u64::MAX);
            let relayed = j["entries_relayed"].as_u64().unwrap_or(0);
            if relayed == 0 { return None; }
            Some((j["operator"].as_str()?.to_owned(), mempool_relay_score(relayed, latency)))
        }).collect();

    // ── Layer B: calibration-normalized utilization + activity-gating + scarcity ─
    let inference_total: u64 = mines.iter().map(|(_, s)| s).sum();
    let storage_total:   u64 = storage_nodes.iter().map(|(_, s)| s).sum();
    let sensor_total:    u64 = sensor_nodes.iter().map(|(_, s)| s).sum();
    let verify_total:    u64 = verifiers.iter().map(|(_, s)| s).sum();
    let service_total:   u64 = service_nodes.iter().map(|(_, s)| s).sum();
    let mempool_total:   u64 = mempool_ops.iter().map(|(_, s)| s).sum();

    let norm = |work: u64, target: u64| -> u64 {
        if work == 0 { return 0; }
        ((work as u128 * ACTIVITY_RATIO_DENOM as u128) / target as u128)
            .min(ACTIVITY_RATIO_DENOM as u128) as u64
    };
    let util_inference = norm(inference_total, CALIBRATION_INFERENCE);
    let util_storage   = norm(storage_total,   CALIBRATION_STORAGE);
    let util_sensor    = norm(sensor_total,     CALIBRATION_SENSOR);
    let util_verify    = norm(verify_total,     CALIBRATION_VERIFIER);
    let util_service   = norm(service_total,    CALIBRATION_SERVICE);
    let util_mempool   = norm(mempool_total,    CALIBRATION_MEMPOOL);
    let total_util     = util_inference + util_storage + util_sensor
                       + util_verify + util_service + util_mempool;

    let active_pool_count = [util_inference, util_storage, util_sensor,
                             util_verify, util_service, util_mempool]
        .iter().filter(|&&u| u > 0).count() as u64;
    let activity_ratio_num = if total_util == 0 {
        0
    } else {
        (total_util / active_pool_count).max(MIN_ACTIVITY_RATIO_NUM)
    };
    let gated_pool   = (activity_pool as u128 * activity_ratio_num as u128 / ACTIVITY_RATIO_DENOM as u128) as u64;
    let idle_recycle = activity_pool.saturating_sub(gated_pool);
    if idle_recycle > 0 { let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, idle_recycle); }

    let alloc_pool = |util: u64| -> u64 {
        if total_util == 0 { return 0; }
        (gated_pool as u128 * util as u128 / total_util as u128) as u64
    };

    let payout_factor = |count: usize, critical_mass: u64| -> u64 {
        ((count as u64).min(critical_mass) * ACTIVITY_RATIO_DENOM / critical_mass.max(1))
    };
    let apply_scarcity = |pool: u64, factor: u64| -> (u64, u64) {
        let effective = (pool as u128 * factor as u128 / ACTIVITY_RATIO_DENOM as u128) as u64;
        (effective, pool.saturating_sub(effective))
    };

    let (inference_pool, si) = apply_scarcity(alloc_pool(util_inference), payout_factor(mines.len(), CRITICAL_MASS_INFERENCE));
    let (storage_pool,   ss) = apply_scarcity(alloc_pool(util_storage),   payout_factor(storage_nodes.len(), CRITICAL_MASS_STORAGE));
    let (sensor_pool,    sn) = apply_scarcity(alloc_pool(util_sensor),    payout_factor(sensor_nodes.len(), CRITICAL_MASS_SENSOR));
    let (verify_pool,    sv) = apply_scarcity(alloc_pool(util_verify),    payout_factor(verifiers.len(), CRITICAL_MASS_VERIFIER));
    let (service_pool,   sc) = apply_scarcity(alloc_pool(util_service),   payout_factor(service_nodes.len(), CRITICAL_MASS_SERVICE));
    let (mempool_pool,   sm) = apply_scarcity(alloc_pool(util_mempool),   payout_factor(mempool_ops.len(), CRITICAL_MASS_MEMPOOL));

    let scarcity_recycle = si + ss + sn + sv + sc + sm;
    if scarcity_recycle > 0 { let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, scarcity_recycle); }

    distribute_pool(epoch, &mines, inference_pool, chain, cmd_tx, |miner, amount, ep| {
        LedgerEntry::MineReward { miner, amount, epoch: ep }
    }).await;
    distribute_pool(epoch, &storage_nodes, storage_pool, chain, cmd_tx, |node_id, amount, ep| {
        LedgerEntry::StorageReward { node_id, amount, epoch: ep }
    }).await;
    distribute_pool(epoch, &sensor_nodes, sensor_pool, chain, cmd_tx, |node_id, amount, ep| {
        LedgerEntry::SensorReward { node_id, amount, epoch: ep }
    }).await;
    distribute_pool(epoch, &verifiers, verify_pool, chain, cmd_tx, |node_id, amount, ep| {
        LedgerEntry::VerifierReward { node_id, amount, epoch: ep }
    }).await;
    distribute_pool(epoch, &service_nodes, service_pool, chain, cmd_tx, |node_id, amount, ep| {
        LedgerEntry::ServiceReward { node_id, amount, epoch: ep }
    }).await;
    distribute_pool(epoch, &mempool_ops, mempool_pool, chain, cmd_tx, |operator, amount, ep| {
        LedgerEntry::MempoolReward { operator, amount, epoch: ep }
    }).await;

    let distributed = inference_pool + storage_pool + sensor_pool
                    + verify_pool + service_pool + mempool_pool;
    let remainder   = gated_pool.saturating_sub(distributed).saturating_sub(scarcity_recycle);
    if remainder > 0 { let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, remainder); }

    info!(
        "clock: epoch {} raw={} reserve={} gated={}({:.0}%) scarcity_recycle={} \
         [infer={}/{} store={}/{} sensor={}/{} verify={}/{} svc={}/{} mem={}/{}]",
        epoch, raw_pool, reserve_total, gated_pool,
        activity_ratio_num as f64 / ACTIVITY_RATIO_DENOM as f64 * 100.0,
        scarcity_recycle + idle_recycle,
        inference_pool, mines.len(), storage_pool, storage_nodes.len(),
        sensor_pool, sensor_nodes.len(), verify_pool, verifiers.len(),
        service_pool, service_nodes.len(), mempool_pool, mempool_ops.len(),
    );
}

async fn distribute_pool<F>(
    epoch: u64,
    nodes: &[(String, u64)],
    pool: u64,
    chain: &crate::chain::Chain,
    cmd_tx: &tokio::sync::mpsc::Sender<crate::net::NetCmd>,
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
        let _ = chain.apply_entry(&entry);
        broadcast_entry(&entry, cmd_tx).await;
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

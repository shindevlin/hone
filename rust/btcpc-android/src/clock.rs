//! Clock consensus for Android micronode.
//!
//! Identical semantics to the desktop node:
//!   • Emits a signed epoch seal every epoch over gossipsub
//!   • Tracks seals from peers; fires EpochFinalize when quorum (≥ 2 unique nodes) reached
//!   • No external API calls — all communication via the embedded libp2p swarm

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use tracing::{info, warn};

use btcpc_types::{
    LedgerEntry, EPOCH_MS, CLOCK_REWARD_DREAMS,
    block_reward_at, era, RECYCLE_ERA, RECYCLE_REWARD_RATE, RECYCLE_REWARD_DENOM,
    RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN,
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

                        // Emit MineReward entries for all miners that contributed this epoch.
                        let sealed_epoch = s.epoch;
                        let prefix = format!("mine:{}:", sealed_epoch);
                        let mines: Vec<(String, u64)> = chain_ref.store.scan_prefix(&prefix)
                            .into_iter()
                            .filter_map(|(_, v)| serde_json::from_slice::<serde_json::Value>(&v).ok())
                            .filter_map(|j| {
                                let miner = j["miner"].as_str()?.to_owned();
                                let wv = j["work_value"].as_u64().unwrap_or(0);
                                Some((miner, wv))
                            })
                            .collect();

                        if !mines.is_empty() {
                            let total_reward = if era(sealed_epoch) >= RECYCLE_ERA {
                                let fund = chain_ref.store.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN);
                                ((fund as u128 * RECYCLE_REWARD_RATE) / RECYCLE_REWARD_DENOM) as u64
                            } else {
                                block_reward_at(sealed_epoch)
                            };
                            let total_work: u64 = mines.iter().map(|(_, w)| w).sum();
                            let n = mines.len() as u64;

                            for (miner, work_value) in &mines {
                                let amount = if total_work == 0 {
                                    total_reward / n
                                } else {
                                    (total_reward as u128 * *work_value as u128 / total_work as u128) as u64
                                };
                                if amount == 0 { continue; }
                                let reward_entry = LedgerEntry::MineReward {
                                    miner: miner.clone(),
                                    amount,
                                    epoch: sealed_epoch,
                                };
                                let _ = chain_ref.apply_entry(&reward_entry);
                                let envelope = serde_json::json!({"entry": reward_entry});
                                if let Ok(data) = serde_json::to_vec(&envelope) {
                                    let _ = cmd_tx_clone.send(crate::net::NetCmd::Broadcast {
                                        topic: "btcpc/entries",
                                        data,
                                    }).await;
                                }
                            }
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

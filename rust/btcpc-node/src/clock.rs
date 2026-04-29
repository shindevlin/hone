//! Clock consensus — collects EPOCH_SEAL gossip messages, computes median
//! timestamp across seals, filters outliers, determines quorum (>51%), and
//! emits a `SealedEpoch` event when an epoch is sealed.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{info, warn};

const SEAL_COLLECT_MS: u64 = 5_000;
const OUTLIER_EPOCH_TOLERANCE: u64 = 2;
const EPOCH_MS: u64 = 30_000;
const ISOLATION_EPOCH_THRESHOLD: u64 = 3;
const MIN_QUORUM_FRACTION: f64 = 0.51;

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpochSeal {
    pub epoch_number: u64,
    pub node_id: String,
    pub timestamp: u64,
    pub seal_hash: String,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SealedEpoch {
    pub epoch: u64,
    pub sealed: bool,
    pub quorum: usize,
    pub total_clocks: usize,
    pub outliers: usize,
    pub winner: Option<EpochSeal>,
    pub signing_clocks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockScore {
    pub node_id: String,
    pub score: i64,
    pub outlier_count: u64,
    pub total_seals: u64,
    pub last_seen_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruthStatus {
    pub truth_bearing: bool,
    pub observer: bool,
    pub external_peer_count: usize,
    pub epoch_states: usize,
}

// ── Internal state ────────────────────────────────────────────────────────────

struct EpochState {
    seals: Vec<EpochSeal>,
    resolved: bool,
    winner: Option<EpochSeal>,
    deadline: Instant,
}

struct Inner {
    epoch_states: HashMap<u64, EpochState>,
    clock_scores: HashMap<String, ClockScore>,
    external_peer_count: usize,
    last_external_peer_epoch: u64,
    current_epoch: u64,
    observer_mode: bool,
}

// ── ClockConsensus ────────────────────────────────────────────────────────────

pub struct ClockConsensus {
    inner: Arc<Mutex<Inner>>,
    sealed_tx: broadcast::Sender<SealedEpoch>,
}

impl ClockConsensus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                epoch_states: HashMap::new(),
                clock_scores: HashMap::new(),
                external_peer_count: 0,
                last_external_peer_epoch: 0,
                current_epoch: 0,
                observer_mode: false,
            })),
            sealed_tx: tx,
        }
    }

    /// Subscribe to sealed-epoch events.
    pub fn subscribe(&self) -> broadcast::Receiver<SealedEpoch> {
        self.sealed_tx.subscribe()
    }

    /// Ingest a raw JSON seal message from gossip.
    /// Expected fields: epoch_number, node_id, timestamp, seal_hash, (optional) signature.
    pub fn receive_seal(&self, seal: serde_json::Value) {
        let parsed: EpochSeal = match serde_json::from_value(seal) {
            Ok(s) => s,
            Err(e) => {
                warn!("[clock] failed to parse seal: {}", e);
                return;
            }
        };

        let epoch = parsed.epoch_number;

        {
            let mut inner = self.inner.lock().unwrap();
            let state = inner
                .epoch_states
                .entry(epoch)
                .or_insert_with(|| EpochState {
                    seals: Vec::new(),
                    resolved: false,
                    winner: None,
                    deadline: Instant::now() + Duration::from_millis(SEAL_COLLECT_MS),
                });

            if state.resolved {
                return;
            }

            if !state.seals.iter().any(|s| s.node_id == parsed.node_id) {
                state.seals.push(parsed);
            }
        }

        // Do not early-resolve on receipt — let tick() handle deadlines.
        // This keeps the collection window consistent across peers.
    }

    /// Called every second from the main loop. Resolves any collection windows
    /// whose deadline has passed.
    pub fn tick(&self) {
        let epochs_to_resolve: Vec<u64> = {
            let inner = self.inner.lock().unwrap();
            inner
                .epoch_states
                .iter()
                .filter(|(_, s)| !s.resolved && s.deadline <= Instant::now())
                .map(|(e, _)| *e)
                .collect()
        };

        for epoch in epochs_to_resolve {
            self.resolve_epoch(epoch);
        }
    }

    /// Set the current epoch (called by net/sync modules when chain advances).
    pub fn set_current_epoch(&self, epoch: u64) {
        self.inner.lock().unwrap().current_epoch = epoch;
    }

    /// Update the number of external (non-loopback) peers seen by the net layer.
    pub fn update_peers(&self, count: usize) {
        let mut inner = self.inner.lock().unwrap();
        inner.external_peer_count = count;

        if count > 0 {
            inner.last_external_peer_epoch = inner.current_epoch;
        }

        let isolated = inner
            .current_epoch
            .saturating_sub(inner.last_external_peer_epoch)
            > ISOLATION_EPOCH_THRESHOLD;

        if isolated != inner.observer_mode {
            inner.observer_mode = isolated;
            if isolated {
                info!(
                    "[clock] no external peers for {} epochs — observer mode",
                    ISOLATION_EPOCH_THRESHOLD
                );
            } else {
                info!("[clock] external peers restored — resuming seal production");
            }
        }
    }

    /// Returns clock node scores as JSON array.
    pub fn get_scores(&self) -> Vec<serde_json::Value> {
        let inner = self.inner.lock().unwrap();
        inner
            .clock_scores
            .values()
            .map(|s| serde_json::to_value(s).unwrap_or(serde_json::Value::Null))
            .collect()
    }

    /// Returns observer/truth-bearing status as JSON.
    pub fn truth_status(&self) -> serde_json::Value {
        let inner = self.inner.lock().unwrap();
        let status = TruthStatus {
            truth_bearing: !inner.observer_mode && inner.external_peer_count > 0,
            observer: inner.observer_mode,
            external_peer_count: inner.external_peer_count,
            epoch_states: inner.epoch_states.len(),
        };
        serde_json::to_value(status).unwrap_or(serde_json::Value::Null)
    }

    /// Inspect the collected state for a specific epoch (for diagnostics/API).
    pub fn get_epoch_state(&self, epoch: u64) -> Option<serde_json::Value> {
        let inner = self.inner.lock().unwrap();
        inner.epoch_states.get(&epoch).map(|s| {
            serde_json::json!({
                "epoch": epoch,
                "resolved": s.resolved,
                "seal_count": s.seals.len(),
                "winner": s.winner,
            })
        })
    }

    // ── Resolution ────────────────────────────────────────────────────────────

    fn resolve_epoch(&self, epoch: u64) {
        let result: Option<SealedEpoch> = {
            let mut inner = self.inner.lock().unwrap();

            // Read fields we need without holding a mutable borrow past their last use.
            // external_peer_count is a plain copy — read before the mut borrow on epoch_states.
            let external_peer_count = inner.external_peer_count;

            // Scope the mutable borrow of epoch_states so it is released before
            // we call inner.update_clock_score() which also needs &mut inner.
            let seals: Vec<EpochSeal> = {
                let state = match inner.epoch_states.get_mut(&epoch) {
                    Some(s) if !s.resolved => s,
                    _ => return,
                };
                state.seals.clone()
                // state (and its mutable borrow of inner) dropped here.
            };

            // Helper: mark the epoch resolved once we've made a final decision.
            // Called just before returning Some(...).
            macro_rules! mark_resolved {
                () => {
                    if let Some(s) = inner.epoch_states.get_mut(&epoch) {
                        s.resolved = true;
                    }
                };
            }

            if seals.is_empty() {
                mark_resolved!();
                info!("[clock] epoch {}: no seals — skipping", epoch);
                Some(SealedEpoch {
                    epoch,
                    sealed: false,
                    quorum: 0,
                    total_clocks: 0,
                    outliers: 0,
                    winner: None,
                    signing_clocks: vec![],
                })
            } else if seals.len() == 1 {
                if external_peer_count > 0 {
                    // Have live peers but only one seal — not enough for quorum yet.
                    return;
                }
                let winner = seals[0].clone();
                inner.update_clock_score(&winner.node_id, true);
                mark_resolved!();
                info!("[clock] epoch {}: self-sealed (single clock, isolated)", epoch);
                Some(SealedEpoch {
                    epoch,
                    sealed: true,
                    quorum: 1,
                    total_clocks: 1,
                    outliers: 0,
                    winner: Some(winner.clone()),
                    signing_clocks: vec![winner.node_id],
                })
            } else {
                let tolerance_ms = OUTLIER_EPOCH_TOLERANCE * EPOCH_MS;
                let mut timestamps: Vec<u64> = seals.iter().map(|s| s.timestamp).collect();
                timestamps.sort_unstable();
                let median = timestamps[timestamps.len() / 2];

                let inliers: Vec<EpochSeal> = seals.iter()
                    .filter(|s| (s.timestamp as i64 - median as i64).unsigned_abs() <= tolerance_ms)
                    .cloned().collect();
                let outliers: Vec<EpochSeal> = seals.iter()
                    .filter(|s| (s.timestamp as i64 - median as i64).unsigned_abs() > tolerance_ms)
                    .cloned().collect();

                for s in &outliers {
                    inner.update_clock_score(&s.node_id, false);
                    warn!("[clock] epoch {}: outlier clock {} (dev {}s)",
                        epoch, s.node_id,
                        (s.timestamp as i64 - median as i64).unsigned_abs() / 1_000);
                }

                // Quorum is >51% of ALL seals received, not just inliers.
                let quorum_needed = std::cmp::max(
                    1,
                    (seals.len() as f64 * MIN_QUORUM_FRACTION).ceil() as usize,
                );
                if inliers.len() < quorum_needed {
                    warn!("[clock] epoch {}: insufficient quorum ({} inliers / {} total, need {})",
                        epoch, inliers.len(), seals.len(), quorum_needed);
                    // Leave resolved=false so more seals can arrive.
                    return;
                }

                let mut hash_count: HashMap<&str, usize> = HashMap::new();
                for s in &inliers {
                    *hash_count.entry(s.seal_hash.as_str()).or_default() += 1;
                }
                let winner_hash = hash_count.iter().max_by_key(|(_, c)| *c)
                    .map(|(h, _)| *h).unwrap_or("");

                let winner_seals: Vec<EpochSeal> = inliers.iter()
                    .filter(|s| s.seal_hash == winner_hash).cloned().collect();

                for s in &winner_seals {
                    inner.update_clock_score(&s.node_id, true);
                }
                for s in inliers.iter().filter(|s| s.seal_hash != winner_hash) {
                    inner.update_clock_score(&s.node_id, false);
                }

                mark_resolved!();
                let winner = winner_seals[0].clone();
                info!("[clock] epoch {} sealed: quorum={}/{} outliers={}",
                    epoch, winner_seals.len(), seals.len(), outliers.len());

                Some(SealedEpoch {
                    epoch,
                    sealed: true,
                    quorum: winner_seals.len(),
                    total_clocks: seals.len(),
                    outliers: outliers.len(),
                    winner: Some(winner.clone()),
                    signing_clocks: winner_seals.iter().map(|s| s.node_id.clone()).collect(),
                })
            }
        };

        // Prune epoch states older than 20 epochs behind the one just resolved.
        {
            let mut inner = self.inner.lock().unwrap();
            inner.epoch_states.retain(|e, _| *e + 20 >= epoch);
        }

        if let Some(event) = result {
            let _ = self.sealed_tx.send(event);
        }
    }
}

impl Default for ClockConsensus {
    fn default() -> Self {
        Self::new()
    }
}

// ── Score helpers ─────────────────────────────────────────────────────────────

impl Inner {
    fn update_clock_score(&mut self, node_id: &str, agreed: bool) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let s = self
            .clock_scores
            .entry(node_id.to_string())
            .or_insert(ClockScore {
                node_id: node_id.to_string(),
                score: 100,
                outlier_count: 0,
                total_seals: 0,
                last_seen_ms: 0,
            });

        s.total_seals += 1;
        s.last_seen_ms = now_ms;

        if agreed {
            s.score = (s.score + 2).min(200);
        } else {
            s.outlier_count += 1;
            s.score = (s.score - 10).max(0);
        }
    }
}

//! Clock consensus — collects EPOCH_SEAL gossip messages, computes median
//! timestamp across seals, filters outliers, determines quorum (>51%), and
//! emits a `SealedEpoch` event when an epoch is sealed.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::store::Store;

const SEAL_COLLECT_MS: u64 = 5_000;
/// Extra wait after initial deadline when we have live peers but only 1 seal.
const PEER_SEAL_WAIT_MS: u64 = 20_000;
const OUTLIER_EPOCH_TOLERANCE: u64 = 2;
const EPOCH_MS: u64 = 30_000;
const ISOLATION_EPOCH_THRESHOLD: u64 = 3;
const MIN_QUORUM_FRACTION: f64 = 0.51;
const CONSENSUS_COLLECT_MS: u64 = 10_000;

/// Minimum unique sealers required to seal an epoch.
/// Configurable via HONE_CLOCK_QUORUM env var (default 2).
pub fn quorum() -> usize {
    std::env::var("HONE_CLOCK_QUORUM")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2)
}

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpochSeal {
    pub epoch_number: u64,
    pub node_id: String,
    pub timestamp: u64,
    pub seal_hash: String,
    pub signature: Option<String>,
    /// ed25519 public key hex (32 bytes = 64 chars).
    /// When present, used for signature verification instead of node_id,
    /// allowing node_id to be the account name for reward routing.
    #[serde(default)]
    pub pubkey: Option<String>,
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

/// A reward consensus proposal broadcast after reward emission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardProposal {
    pub epoch: u64,
    pub node_id: String,
    pub rewards_hash: String,
}

/// Emitted when 51% of clock nodes agree on rewards for an epoch.
#[derive(Debug, Clone)]
pub struct FinalizedEpoch {
    pub epoch: u64,
    pub rewards_hash: String,
    pub quorum: usize,
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
    /// Extended deadline: if we have 1 seal and live peers, wait until this
    /// before self-sealing. Set to deadline + PEER_SEAL_WAIT_MS on first wait.
    peer_fallback_deadline: Option<Instant>,
}

struct RewardState {
    proposals: Vec<RewardProposal>,
    finalized: bool,
    deadline: Instant,
}

struct Inner {
    epoch_states: HashMap<u64, EpochState>,
    reward_states: HashMap<u64, RewardState>,
    clock_scores: HashMap<String, ClockScore>,
    external_peer_count: usize,
    last_external_peer_epoch: u64,
    current_epoch: u64,
    observer_mode: bool,
    /// Current registered clock node set — updated at each epoch seal via
    /// `set_registered_clocks()`. Used as the quorum denominator.
    registered_clocks: Vec<String>,
}

// ── ClockConsensus ────────────────────────────────────────────────────────────

pub struct ClockConsensus {
    inner: Arc<Mutex<Inner>>,
    sealed_tx: broadcast::Sender<SealedEpoch>,
    finalized_tx: broadcast::Sender<FinalizedEpoch>,
}

impl ClockConsensus {
    pub fn new() -> Self {
        let (sealed_tx, _)    = broadcast::channel(64);
        let (finalized_tx, _) = broadcast::channel(64);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                epoch_states: HashMap::new(),
                reward_states: HashMap::new(),
                clock_scores: HashMap::new(),
                external_peer_count: 0,
                last_external_peer_epoch: 0,
                current_epoch: 0,
                observer_mode: false,
                registered_clocks: Vec::new(),
            })),
            sealed_tx,
            finalized_tx,
        }
    }

    /// Subscribe to sealed-epoch events.
    pub fn subscribe(&self) -> broadcast::Receiver<SealedEpoch> {
        self.sealed_tx.subscribe()
    }

    /// Subscribe to finalized-epoch events (reward quorum reached).
    pub fn subscribe_finalized(&self) -> broadcast::Receiver<FinalizedEpoch> {
        self.finalized_tx.subscribe()
    }

    /// Ingest a reward consensus proposal from a peer (or self).
    pub fn receive_reward_proposal(&self, proposal: RewardProposal) {
        let epoch = proposal.epoch;
        let finalized_event = {
            let mut inner = self.inner.lock().unwrap();
            // Read registered_clocks before the mutable entry borrow to satisfy the borrow checker.
            let reg = inner.registered_clocks.clone();
            // Read current epoch too (for the bootstrap solo bypass below).
            let cur_epoch = inner.current_epoch;

            let state = inner.reward_states.entry(epoch).or_insert_with(|| RewardState {
                proposals: Vec::new(),
                finalized: false,
                deadline: Instant::now() + Duration::from_millis(CONSENSUS_COLLECT_MS),
            });

            if state.finalized { return; }

            // Deduplicate per node_id.
            if !state.proposals.iter().any(|p| p.node_id == proposal.node_id) {
                state.proposals.push(proposal);
            }

            // Filter proposals to registered nodes when the set is known.
            // Unregistered nodes cannot influence reward finalization.
            let valid_proposals: Vec<&RewardProposal> = if reg.is_empty() {
                state.proposals.iter().collect()
            } else {
                state.proposals.iter().filter(|p| reg.contains(&p.node_id)).collect()
            };

            // Denominator is the registered set size when known; else observed proposals.
            let denominator = if reg.is_empty() { state.proposals.len() } else { reg.len() };
            let quorum_needed = {
                let normal = ((denominator as f64 * MIN_QUORUM_FRACTION).ceil() as usize)
                    .max(quorum());
                // Bootstrap solo bypass (BUG 6): a genuinely solo registered clock
                // (denominator <= 1) can never reach the default quorum of 2, so it would
                // seal via the sealing bootstrap escape hatch yet NEVER finalize rewards —
                // silently earning nothing on single-node / early-genesis deployments. During
                // the bootstrap grace window, mirror the sealing bypass: a lone clock may
                // finalize with just its own proposal. Outside grace the normal quorum holds.
                let in_grace = cur_epoch <= hone_types::CLOCK_BOOTSTRAP_GRACE_END_EPOCH;
                if denominator <= 1 && in_grace { 1 } else { normal }
            };

            // Tally votes from valid (registered) proposals only.
            let mut hash_counts: HashMap<String, usize> = HashMap::new();
            for p in &valid_proposals {
                *hash_counts.entry(p.rewards_hash.clone()).or_default() += 1;
            }

            if let Some((best_hash, &count)) = hash_counts.iter().max_by_key(|(_, c)| *c) {
                if count >= quorum_needed {
                    state.finalized = true;
                    Some(FinalizedEpoch {
                        epoch,
                        rewards_hash: best_hash.clone(),
                        quorum: count,
                    })
                } else { None }
            } else { None }
        };

        if let Some(event) = finalized_event {
            info!("[clock] epoch {} reward consensus: quorum={} hash={}", event.epoch, event.quorum, event.rewards_hash);
            let _ = self.finalized_tx.send(event);
        }
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

        // Verify ed25519 signature when a pubkey is available.
        // pubkey field takes priority; fall back to node_id if it looks like a raw pubkey hex.
        // Nodes without a posting key send signature=null — allowed.
        if let Some(ref sig_hex) = parsed.signature {
            let key_hex = parsed.pubkey.as_deref()
                .or_else(|| if parsed.node_id.len() == 64 { Some(&parsed.node_id) } else { None });
            if let Some(kh) = key_hex {
                if !verify_seal_signature(&parsed, kh, sig_hex) {
                    warn!("[clock] rejected seal from {} — bad signature", parsed.node_id);
                    return;
                }
            }
        }

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
                    peer_fallback_deadline: None,
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

    /// Ensure an `epoch_states` entry exists for `epoch` so it will be resolved by
    /// `tick()` even if NO seal is ever received for it (a genuinely empty epoch).
    ///
    /// Negative attestation (BUG 6): empty epochs previously got no epoch_states entry
    /// (entries were created only on receive_seal), so they were never resolved, never
    /// finalized, and only the per-node local finalizer timer credited their recycle —
    /// diverging state between nodes. By seeding an entry here for every elapsed epoch,
    /// an empty epoch is resolved as `sealed:false` on EVERY node, broadcasts a reward
    /// proposal, and reaches the SAME quorum-agreed FinalizedEpoch everywhere — so the
    /// recycle/decay that hangs off finalization runs deterministically. Idempotent: a
    /// later seal for the epoch simply appends to the existing entry.
    pub fn ensure_epoch_tracked(&self, epoch: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.epoch_states.entry(epoch).or_insert_with(|| EpochState {
            seals: Vec::new(),
            resolved: false,
            winner: None,
            deadline: Instant::now() + Duration::from_millis(SEAL_COLLECT_MS),
            peer_fallback_deadline: None,
        });
    }

    /// Update the registered clock node set. Called at each epoch seal from main.rs.
    /// When this list is non-empty it becomes the quorum denominator (>51% of registered nodes).
    pub fn set_registered_clocks(&self, nodes: Vec<String>) {
        self.inner.lock().unwrap().registered_clocks = nodes;
    }

    /// Return the current registered clock node list (for API/diagnostics).
    pub fn registered_clocks(&self) -> Vec<String> {
        self.inner.lock().unwrap().registered_clocks.clone()
    }

    /// Update the number of external (non-loopback) peers seen by the net layer.
    pub fn update_peers(&self, count: usize) {
        let mut inner = self.inner.lock().unwrap();
        inner.external_peer_count = count;

        if count > 0 {
            inner.last_external_peer_epoch = inner.current_epoch;
        }

        let isolated = inner.external_peer_count == 0
            && inner
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
            let registered_clocks = inner.registered_clocks.clone();

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

            // Bootstrap master: during explicit isolation flag OR when there are no peers
            // and no registered clock nodes (true genesis bootstrap).
            // With live peers present, fall through to the quorum path so multi-node
            // networks seal properly even before ClockRegister transactions exist.
            let bootstrap_isolation = std::env::var("HONE_BOOTSTRAP_ISOLATION")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false);
            let bootstrap_master = "shindevlin";
            let bootstrap_seal = if bootstrap_isolation
                || (registered_clocks.is_empty() && external_peer_count == 0)
            {
                seals.iter().find(|s| s.node_id == bootstrap_master).cloned()
            } else {
                None
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

            if let Some(master_seal) = bootstrap_seal {
                inner.update_clock_score(bootstrap_master, true);
                mark_resolved!();
                info!("[clock] epoch {}: bootstrap seal by '{}'", epoch, bootstrap_master);
                Some(SealedEpoch {
                    epoch,
                    sealed: true,
                    quorum: 1,
                    total_clocks: seals.len(),
                    outliers: 0,
                    winner: Some(master_seal.clone()),
                    signing_clocks: vec![master_seal.node_id],
                })
            } else if seals.is_empty() {
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
                    // Have live peers but only one seal — wait up to PEER_SEAL_WAIT_MS
                    // for a peer seal to arrive before self-sealing.
                    let state = inner.epoch_states.get_mut(&epoch).unwrap();
                    let fallback = state.peer_fallback_deadline.get_or_insert_with(|| {
                        Instant::now() + Duration::from_millis(PEER_SEAL_WAIT_MS)
                    });
                    if Instant::now() < *fallback {
                        return; // still waiting
                    }
                    // Fallback expired — self-seal despite having peers.
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

                // When the registered set is known, only registered seals count toward quorum.
                // Unregistered seals contributed to the timestamp median above but cannot vote.
                let voting_inliers: Vec<EpochSeal> = if registered_clocks.is_empty() {
                    inliers.clone()
                } else {
                    inliers.iter()
                        .filter(|s| registered_clocks.contains(&s.node_id))
                        .cloned()
                        .collect()
                };

                let denominator = if registered_clocks.is_empty() {
                    seals.len()
                } else {
                    registered_clocks.len()
                };
                let quorum_needed = std::cmp::max(
                    1,
                    (denominator as f64 * MIN_QUORUM_FRACTION).ceil() as usize,
                );
                if voting_inliers.len() < quorum_needed {
                    warn!(
                        "[clock] epoch {}: insufficient quorum ({} registered inliers / {} registered, need {}; {} total seals received)",
                        epoch, voting_inliers.len(), denominator, quorum_needed, seals.len()
                    );
                    return;
                }

                // Among voting inliers, pick the winning seal_hash (most common).
                let mut hash_count: HashMap<&str, usize> = HashMap::new();
                for s in &voting_inliers {
                    *hash_count.entry(s.seal_hash.as_str()).or_default() += 1;
                }
                let winner_hash = hash_count.iter().max_by_key(|(_, c)| *c)
                    .map(|(h, _)| *h).unwrap_or("");

                let winner_seals: Vec<EpochSeal> = voting_inliers.iter()
                    .filter(|s| s.seal_hash == winner_hash).cloned().collect();

                for s in &winner_seals {
                    inner.update_clock_score(&s.node_id, true);
                }
                for s in voting_inliers.iter().filter(|s| s.seal_hash != winner_hash) {
                    inner.update_clock_score(&s.node_id, false);
                }
                // Non-voting outliers (unregistered or timestamp outliers) get scored down.
                for s in &outliers {
                    inner.update_clock_score(&s.node_id, false);
                }

                mark_resolved!();
                let winner = winner_seals[0].clone();
                info!(
                    "[clock] epoch {} sealed: quorum={}/{} registered, {} total seals, {} outliers",
                    epoch, winner_seals.len(), denominator, seals.len(), outliers.len()
                );

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

// ── Reward hash ───────────────────────────────────────────────────────────────

/// Compute a deterministic SHA-256 hash of all epoch input state (sorted by key).
/// Used by clock nodes to propose and verify reward fairness.
pub fn compute_rewards_hash(epoch: u64, store: &Store) -> String {
    let mut hasher = Sha256::new();
    let prefixes = [
        format!("mine:{}:", epoch),
        format!("storage_beat:{}:", epoch),
        format!("sensor_commit:{}:", epoch),
        format!("tracker_sighting:{}:", epoch),
        format!("infer_verify:{}:", epoch),
        format!("service_beat:{}:", epoch),
        format!("mempool_beat:{}:", epoch),
    ];
    for prefix in &prefixes {
        let mut entries = store.state_scan_prefix(prefix);
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

// ── Registered clock nodes ────────────────────────────────────────────────────

/// Return all nodes eligible for the clock quorum.
///
/// Eligibility is pool-driven (FIFO staking): a node is in quorum if its
/// self-stake in `role_stake:clock:{node}:{node}` is >= `clock_min_stake`.
/// No explicit `ClockNodeRegister` transaction is required — staking IS
/// registration. Nodes are ordered by the epoch they first staked (FIFO);
/// if stake later drops below the minimum, the node loses its slot.
///
/// `clock_reg:` entries are still respected for backward-compat and pubkey
/// caching, but the authoritative eligibility signal is the live pool stake.
/// Slashed nodes (pool zeroed via ClockDoubleSignEvidence) are excluded.
pub fn registered_clock_nodes(store: &Store, current_epoch: u64) -> Vec<String> {
    let min_stake: u64 = store.state_get("chain_param:clock_min_stake")
        .and_then(|b| serde_json::from_slice::<u64>(&b).ok())
        .unwrap_or(5 * 10_000_000_000);

    // Bootstrap grace: during the first CLOCK_BOOTSTRAP_GRACE_END_EPOCH epochs a
    // founder clock registers at stake 0 (the POW-genesis deadlock-breaker) and has
    // no role_stake yet. The quorum denominator MUST include these grace clocks, or
    // two founder clocks each stay a solo 1/1 quorum and the chain never advances
    // past genesis with real 2-of-2 consensus. Outside grace the stake rules apply.
    // See docs/CLOCK_BOOTSTRAP_GRACE.md + DRYRUN_2CLOCK — consensus-critical.
    let in_grace = current_epoch <= hone_types::CLOCK_BOOTSTRAP_GRACE_END_EPOCH;

    // Aggregate total stake per node across ALL stakers (self + backers).
    // Key format: role_stake:clock:{node}:{staker}
    // A node enters quorum when its total backing reaches min_stake.
    // FIFO: earliest first-stake epoch per node gets priority.
    let mut per_node: std::collections::HashMap<String, (u64, u64)> = std::collections::HashMap::new();
    for (key, val) in store.state_scan_prefix("role_stake:clock:") {
        let rest = match key.strip_prefix("role_stake:clock:") {
            Some(r) => r.to_owned(),
            None => continue,
        };
        let sep = match rest.rfind(':') {
            Some(i) => i,
            None => continue,
        };
        let node = rest[..sep].to_owned();
        let j: serde_json::Value = match serde_json::from_slice(&val) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let amount = j["amount"].as_u64().unwrap_or(0);
        let staked_epoch = j["staked_epoch"].as_u64().unwrap_or(u64::MAX);
        let entry = per_node.entry(node).or_insert((0, u64::MAX));
        entry.0 += amount;
        entry.1 = entry.1.min(staked_epoch); // earliest stake epoch wins
    }

    let mut eligible: Vec<(String, u64)> = per_node
        .into_iter()
        .filter(|(_, (total, _))| *total >= min_stake)
        .map(|(node, (_, first_epoch))| (node, first_epoch))
        .collect();

    // FIFO: nodes that staked earlier get priority.
    eligible.sort_by_key(|(_, epoch)| *epoch);
    let mut nodes: Vec<String> = eligible.into_iter().map(|(id, _)| id).collect();

    // Also include any legacy clock_reg: entries that aren't already in the list
    // (e.g. genesis-era registrations that pre-date the pool model) — only if
    // the pool stake still covers them or they have a non-zero legacy stake.
    for (key, val) in store.state_scan_prefix("clock_reg:") {
        let node_id = match key.strip_prefix("clock_reg:") {
            Some(n) => n.to_owned(),
            None => continue,
        };
        if nodes.contains(&node_id) { continue; }
        let j: serde_json::Value = match serde_json::from_slice(&val) {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Non-zero stake = the stake was balance-deducted at registration time.
        // During bootstrap grace we ALSO include stake-0 registrations, since grace
        // clocks legitimately register at 0 (they build stake from ClockReward). This
        // is what makes 2 grace-registered founder clocks count as a 2-of-2 quorum.
        if j["stake"].as_u64().unwrap_or(0) > 0 || in_grace {
            nodes.push(node_id);
        }
    }

    nodes.sort();
    nodes.dedup();
    nodes
}

// ── Epoch entropy ─────────────────────────────────────────────────────────────

/// Compute epoch entropy = SHA-256 of the XOR of all winning seal hashes for the epoch.
/// Stored in sled as `epoch_entropy:{epoch}` after each epoch seal.
/// Used for randomness in future selection protocols.
pub fn compute_epoch_entropy(seal_hashes: &[&str]) -> String {
    // XOR all seal hashes as 32-byte arrays.
    let mut xor_state = [0u8; 32];
    for hash_hex in seal_hashes {
        if let Ok(bytes) = hex::decode(hash_hex) {
            let bytes: Vec<u8> = bytes.into_iter().take(32).collect();
            for (i, b) in bytes.iter().enumerate() {
                xor_state[i] ^= b;
            }
        }
    }
    let mut hasher = Sha256::new();
    hasher.update(&xor_state);
    format!("{:x}", hasher.finalize())
}

/// Derive the per-epoch Merkle range challenge for a storage node (T4-1, D10).
///
/// challenge = sha256("{seal_hash}:{node_id}:{epoch}")
///
/// Deterministic from public data — any peer can recompute and verify.
/// Storage nodes include the matching `challenge_hash` in their `StorageHeartbeat`
/// along with a Merkle proof over the byte range identified by the challenge.
pub fn compute_storage_challenge(seal_hash: &str, node_id: &str, epoch: u64) -> String {
    let mut h = Sha256::new();
    h.update(format!("{}:{}:{}", seal_hash, node_id, epoch).as_bytes());
    format!("{:x}", h.finalize())
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

// ── Seal signature verification ───────────────────────────────────────────────

/// Verify an ed25519 seal signature over the canonical message
/// `"seal:{epoch}:{seal_hash}:{node_id}:{timestamp}"`.
/// Used by both gossip ingestion and double-sign slash evidence verification.
pub fn verify_clock_seal_sig(
    epoch: u64,
    seal_hash: &str,
    node_id: &str,
    timestamp: u64,
    pubkey_hex: &str,
    sig_hex: &str,
) -> bool {
    use ed25519_dalek::{VerifyingKey, Verifier};
    let pk_bytes = match hex::decode(pubkey_hex) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let arr: [u8; 32] = match pk_bytes.try_into() {
        Ok(a) => a,
        Err(_) => return false,
    };
    let vk = match VerifyingKey::from_bytes(&arr) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let sig_bytes = match hex::decode(sig_hex) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let sig_arr: [u8; 64] = match sig_bytes.try_into() {
        Ok(a) => a,
        Err(_) => return false,
    };
    let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);
    let msg = format!("seal:{}:{}:{}:{}", epoch, seal_hash, node_id, timestamp);
    vk.verify(msg.as_bytes(), &sig).is_ok()
}

fn verify_seal_signature(seal: &EpochSeal, pubkey_hex: &str, sig_hex: &str) -> bool {
    verify_clock_seal_sig(
        seal.epoch_number, &seal.seal_hash, &seal.node_id, seal.timestamp,
        pubkey_hex, sig_hex,
    )
}

#[cfg(test)]
mod registered_clock_nodes_tests {
    use super::*;

    fn store() -> (crate::store::Store, tempfile::TempDir) {
        let dir = tempfile::Builder::new().prefix("hone_clock_reg_").tempdir().unwrap();
        let s = crate::store::Store::open(dir.path()).unwrap();
        (s, dir)
    }

    fn register(s: &crate::store::Store, node: &str, stake: u64) {
        let rec = serde_json::json!({
            "node_id": node, "stake": stake, "registered_epoch": 0, "pubkey": "aa"
        });
        s.state_set(&format!("clock_reg:{}", node), &serde_json::to_vec(&rec).unwrap()).unwrap();
    }

    // Bug 5 guard: during bootstrap grace, two stake-0 founder clocks must BOTH count
    // toward quorum. Before the fix each stake-0 clock was excluded, so two clocks each
    // computed a solo 1/1 quorum and the chain never advanced past genesis.
    #[test]
    fn grace_includes_stake_zero_clocks() {
        let (s, _d) = store();
        register(&s, "clocka", 0);
        register(&s, "clockb", 0);
        let in_grace = hone_types::CLOCK_BOOTSTRAP_GRACE_END_EPOCH; // last grace epoch
        let nodes = registered_clock_nodes(&s, in_grace);
        assert!(nodes.contains(&"clocka".to_string()), "clocka must count in grace");
        assert!(nodes.contains(&"clockb".to_string()), "clockb must count in grace");
        assert_eq!(nodes.len(), 2, "both grace clocks form a 2-of-2 quorum");
    }

    // After grace, a stake-0 registration with no role_stake is NOT quorum-eligible —
    // the normal minimum-stake rule applies. (Prevents free post-grace quorum seats.)
    #[test]
    fn post_grace_excludes_stake_zero_clocks() {
        let (s, _d) = store();
        register(&s, "clocka", 0);
        let after_grace = hone_types::CLOCK_BOOTSTRAP_GRACE_END_EPOCH + 1;
        let nodes = registered_clock_nodes(&s, after_grace);
        assert!(!nodes.contains(&"clocka".to_string()),
            "a stake-0 clock must NOT count once grace has ended");
    }

    // A non-zero legacy clock_reg stake counts regardless of grace (unchanged behavior).
    #[test]
    fn nonzero_stake_counts_regardless_of_grace() {
        let (s, _d) = store();
        register(&s, "clocka", 50_000_000_000);
        let after_grace = hone_types::CLOCK_BOOTSTRAP_GRACE_END_EPOCH + 1;
        let nodes = registered_clock_nodes(&s, after_grace);
        assert!(nodes.contains(&"clocka".to_string()));
    }
}

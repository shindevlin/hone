//! Clock consensus — collects EPOCH_SEAL gossip messages, computes median
//! timestamp across seals, filters outliers, determines quorum (>51%), and
//! emits a `SealedEpoch` event when an epoch is sealed.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{debug, info, warn};

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

// ── Reputation-weighted quorum (v1) ─────────────────────────────────────────────
// See docs/REPUTATION_WEIGHTED_QUORUM.md. Weight = stake_term · uptime_term, BOTH on-chain
// (role_stake:clock, clock_uptime:{node}). INTEGER-ONLY end to end — floats in the sum or
// comparison path fork a cross-arch cohort by one ULP (Beastly review 940a6577), and the
// outlier term was dropped because clock_scores is in-memory + wall-clock + local (a fork).
// Every function here is a pure function of on-chain inputs → restart-invariant by
// construction (the §9 key assertion): no wall-clock, no local state, no floats.

const CLOCK_WEIGHT_MIN_EPOCHS: u64 = 10;              // probation: below this, weight = 0
const CLOCK_WEIGHT_STAKE_BASE: u128 = 100;           // floor so an established 0-stake clock still votes
const CLOCK_WEIGHT_STAKE_GRANULE: u64 = 10_000_000_000; // 1 HONE — isqrt is taken over whole-HONE units

/// Integer square root of a u128 (Newton's method). Bit-exact on every architecture —
/// this is what replaces the spec's rejected `sqrt`/`log1p`.
pub fn isqrt_u128(n: u128) -> u128 {
    if n < 2 { return n; }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Per-clock consensus weight — integer, deterministic, on-chain inputs only.
///
/// `weight = uptime_permille · stake_term`
///   uptime_permille = seals·1000/epochs  ∈ 0..=1000   (from clock_uptime:{node})
///   stake_term      = STAKE_BASE + isqrt(stake / 1 HONE)   (diminishing returns, §4)
///
/// Probation (§3): a clock registered fewer than CLOCK_WEIGHT_MIN_EPOCHS epochs returns 0 —
/// it seals and builds uptime but carries no quorum weight until it has proven presence.
pub fn clock_weight(seals: u64, epochs: u64, stake_hunits: u64) -> u128 {
    if epochs < CLOCK_WEIGHT_MIN_EPOCHS {
        return 0;
    }
    let uptime_permille = (seals.min(epochs) as u128 * 1000) / (epochs as u128); // 0..=1000
    let stake_term =
        CLOCK_WEIGHT_STAKE_BASE + isqrt_u128((stake_hunits / CLOCK_WEIGHT_STAKE_GRANULE) as u128);
    uptime_permille * stake_term
}

/// Weighted-quorum decision. Given votes `(node_id, hash)`, the registered set, and each
/// registered clock's weight, return the winning hash iff its summed (capped) weight exceeds
/// 51% of the total (capped) registered weight AND its vote count meets `floor`.
///
/// Anti-concentration (§6): each clock is capped at 1/3 of the PRE-CAP total, no
/// renormalization (Beastly review — renorm iteration in the seal path is a liveness hazard).
/// If the total registered weight is 0 (e.g. every clock still in probation), returns None so
/// the caller falls back to the flat count rule — weighted quorum never stalls a fresh cohort.
///
/// Integer-only; deterministic tie-break (weight, then count, then hash bytes) so every node
/// picks the identical winner.
pub fn weighted_winner(
    votes: &[(String, String)],
    registered: &[String],
    weights: &HashMap<String, u128>,
    floor: usize,
) -> Option<(String, usize, u128)> {
    let raw_total: u128 = registered.iter().map(|n| *weights.get(n).unwrap_or(&0)).sum();
    if raw_total == 0 {
        return None; // all probation / no weight yet → caller uses flat count
    }
    let cap = raw_total / 3;
    let capped = |n: &str| -> u128 { (*weights.get(n).unwrap_or(&0)).min(cap) };
    let total: u128 = registered.iter().map(|n| capped(n)).sum();

    let mut by_hash: HashMap<&str, (usize, u128)> = HashMap::new();
    for (node, hash) in votes {
        // Only registered clocks contribute weight; unregistered votes are ignored.
        if !registered.iter().any(|r| r == node) {
            continue;
        }
        let e = by_hash.entry(hash.as_str()).or_insert((0, 0));
        e.0 += 1;
        e.1 += capped(node);
    }
    let winner = by_hash
        .iter()
        .max_by(|a, b| {
            a.1 .1
                .cmp(&b.1 .1)
                .then(a.1 .0.cmp(&b.1 .0))
                .then(a.0.cmp(b.0))
        })
        .map(|(h, (c, w))| (h.to_string(), *c, *w))?;

    // Strict weighted majority: weight·100 > total·51. Integer, no float.
    if winner.2 * 100 > total * 51 && winner.1 >= floor {
        Some(winner)
    } else {
        None
    }
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
    /// Set after the first "insufficient quorum" warning for this epoch, so a
    /// stuck epoch logs once instead of every tick.
    quorum_warned: bool,
}

struct RewardState {
    proposals: Vec<RewardProposal>,
    finalized: bool,
    deadline: Instant,
}

struct Inner {
    epoch_states: HashMap<u64, EpochState>,
    reward_states: HashMap<u64, RewardState>,
    /// Catch-up reward votes for past epochs that never finalized: epoch → node → hash.
    catchup_votes: HashMap<u64, HashMap<String, String>>,
    clock_scores: HashMap<String, ClockScore>,
    external_peer_count: usize,
    last_external_peer_epoch: u64,
    current_epoch: u64,
    observer_mode: bool,
    /// Current registered clock node set — updated at each epoch seal via
    /// `set_registered_clocks()`. Used as the quorum denominator.
    registered_clocks: Vec<String>,
    /// Reputation-weighted quorum (v1), EPOCH-ANCHORED. Maps the epoch being decided →
    /// `(enabled, weights)` that govern it. The decision for epoch `E` uses `epoch_weights[E]`
    /// only — never a mutable "latest" — so every node (and a node before vs. after a restart)
    /// resolves `E` from the identical committed-state anchor. Reading a shared latest field
    /// instead would reintroduce the §3-class bug at the timing layer: a node whose epoch
    /// handler lagged would decide `E` with a stale anchor and fork. Injected per epoch from
    /// main.rs via `set_clock_weights(epoch, …)`, repopulated at startup for the pending epoch.
    /// Absent entry (or disabled / zero total weight) → flat count rule, byte-identical to pre-change.
    epoch_weights: HashMap<u64, (bool, HashMap<String, u128>)>,
}

impl Inner {
    /// True the first time an epoch fails quorum; false on every retry after that.
    fn first_quorum_warn(&mut self, epoch: u64) -> bool {
        match self.epoch_states.get_mut(&epoch) {
            Some(s) if !s.quorum_warned => { s.quorum_warned = true; true }
            _ => false,
        }
    }
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
        // Catch-up can finalize hundreds of past epochs per batch; a 64-slot channel
        // would drop them (the batch is re-voted, but there is no reason to lag).
        let (finalized_tx, _) = broadcast::channel(4096);
        Self {
            inner: Arc::new(Mutex::new(Inner {
                epoch_states: HashMap::new(),
                reward_states: HashMap::new(),
                catchup_votes: HashMap::new(),
                clock_scores: HashMap::new(),
                external_peer_count: 0,
                last_external_peer_epoch: 0,
                current_epoch: 0,
                observer_mode: false,
                registered_clocks: Vec::new(),
                epoch_weights: HashMap::new(),
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
            // Epoch-anchored weights: use THIS epoch's, never a mutable latest.
            let (weighted_quorum_enabled, clock_weights) = inner
                .epoch_weights
                .get(&epoch)
                .cloned()
                .unwrap_or((false, HashMap::new()));

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

            // Filter proposals to registered REWARD VOTERS when the set is known.
            // Unregistered nodes cannot influence reward finalization. Device clock seats
            // ("owner@pubkey", e.g. a phone) seal time and get paid for it, but they never
            // compute or gossip a rewards_hash — counting them in the denominator only made
            // quorum unreachable (4 registered with one phone → 3 of 3 servers had to agree).
            let voters = reward_voters(&reg);
            let valid_proposals: Vec<&RewardProposal> = if reg.is_empty() {
                state.proposals.iter().collect()
            } else {
                state.proposals.iter().filter(|p| voters.contains(&p.node_id)).collect()
            };

            // Denominator is the voter set size when known; else observed proposals.
            let denominator = if reg.is_empty() { state.proposals.len() } else { voters.len() };
            // Rewards require REAL quorum (Shin ruling a2d7f6c9: ">=2 for reward is fine").
            // The earlier bootstrap solo-reward bypass (let a lone clock finalize its own
            // rewards during grace) is REMOVED, not grace-scoped — a grace counter is the
            // exact pattern that produced the permanently-open bypass Grouchly found, and
            // "no bypass" has nothing to misconfigure. A solo node still SEALS (chain
            // advances) but earns nothing until quorum >=2 reforms — at genesis OR under a
            // post-launch partition. Delayed-but-recoverable rewards beat divergent-paid.
            let quorum_needed = ((denominator as f64 * MIN_QUORUM_FRACTION).ceil() as usize)
                .max(quorum());

            // Weighted reward finalization (v1, gated) — kept aligned with the seal decision so
            // an epoch that seals under weighted quorum also finalizes its rewards under the same
            // rule (and vice versa). Same guard: enabled + nonzero registered weight, else flat.
            let reg_weight_total: u128 = reg.iter()
                .map(|n| clock_weights.get(n).copied().unwrap_or(0))
                .sum();
            if weighted_quorum_enabled && !reg.is_empty() && reg_weight_total > 0 {
                let votes: Vec<(String, String)> = valid_proposals.iter()
                    .map(|p| (p.node_id.clone(), p.rewards_hash.clone()))
                    .collect();
                match weighted_winner(&votes, &reg, &clock_weights, quorum())
                    .filter(|(h, _, _)| operators_agree(&valid_proposals, h))
                {
                    Some((best_hash, count, weight)) => {
                        info!(
                            "[clock] epoch {} reward: WEIGHTED finalize — winner weight {} / {} registered ({} signers)",
                            epoch, weight, reg_weight_total, count
                        );
                        state.finalized = true;
                        Some(FinalizedEpoch { epoch, rewards_hash: best_hash, quorum: count })
                    }
                    None => {
                        // Distinguish "weighted rejected the tally" from "no weights present" so a
                        // gate run can't mistake reward finalization silently running flat for a
                        // weighted pass (parity with the seal-path engagement log).
                        debug!(
                            "[clock] epoch {} reward: weighted tally below threshold (total weight {})",
                            epoch, reg_weight_total
                        );
                        None
                    }
                }
            } else {
                // Tally votes from valid (registered) proposals only (flat count rule).
                let mut hash_counts: HashMap<String, usize> = HashMap::new();
                for p in &valid_proposals {
                    *hash_counts.entry(p.rewards_hash.clone()).or_default() += 1;
                }

                if let Some((best_hash, &count)) = hash_counts.iter().max_by_key(|(_, c)| *c) {
                    if count >= quorum_needed && operators_agree(&valid_proposals, best_hash) {
                        state.finalized = true;
                        Some(FinalizedEpoch {
                            epoch,
                            rewards_hash: best_hash.clone(),
                            quorum: count,
                        })
                    } else { None }
                } else { None }
            }
        };

        if let Some(event) = finalized_event {
            info!("[clock] epoch {} reward consensus: quorum={} hash={}", event.epoch, event.quorum, event.rewards_hash);
            let _ = self.finalized_tx.send(event);
        }
    }

    /// Catch-up consensus for PAST epochs that sealed (or were empty) but never got a
    /// quorum EpochFinalize — e.g. while the nodes' ledgers were split, or while the
    /// network was down. The live path only votes at seal time, so such an epoch could
    /// never finalize afterwards and the strictly-contiguous reward driver blocked on it
    /// forever (mainnet: stuck at 74,476 since July).
    ///
    /// Same rule as live reward finalization, decided by the chain, not one node:
    /// voters are the epoch's own registered full-node clocks (its `epoch_validators`
    /// snapshot; the current set for an epoch with no snapshot), >51% and >= quorum()
    /// of them must sign the same rewards_hash, from >= min_reward_operators() distinct
    /// operators. Votes arrive signed (verified by the caller). `voters_for(e)` and
    /// `is_finalized(e)` read committed chain state, so every node tallies identically.
    /// Returns how many epochs reached quorum in this call.
    pub fn receive_catchup_votes<V, F>(
        &self,
        node_id: &str,
        votes: &[(u64, String)],
        voters_for: V,
        is_finalized: F,
    ) -> usize
    where
        V: Fn(u64) -> Vec<String>,
        F: Fn(u64) -> bool,
    {
        let mut events = Vec::new();
        {
            let mut inner = self.inner.lock().unwrap();
            for (epoch, hash) in votes {
                let epoch = *epoch;
                if is_finalized(epoch) {
                    inner.catchup_votes.remove(&epoch);
                    continue;
                }
                let voters = voters_for(epoch);
                if !voters.iter().any(|v| v == node_id) {
                    continue;
                }
                let tally = inner.catchup_votes.entry(epoch).or_default();
                tally.insert(node_id.to_owned(), hash.clone());

                let mut counts: HashMap<&str, Vec<&str>> = HashMap::new();
                for (n, h) in tally.iter() {
                    if voters.contains(n) {
                        counts.entry(h.as_str()).or_default().push(n.as_str());
                    }
                }
                let needed = ((voters.len() as f64 * MIN_QUORUM_FRACTION).ceil() as usize)
                    .max(quorum());
                let winner = counts.iter()
                    .filter(|(_, ns)| ns.len() >= needed)
                    .filter(|(_, ns)| {
                        ns.iter().map(|n| clock_operator(n)).collect::<HashSet<_>>().len()
                            >= min_reward_operators()
                    })
                    // Deterministic pick if two hashes both qualify (cannot happen with
                    // >51%, but never let HashMap order decide).
                    .max_by(|a, b| a.1.len().cmp(&b.1.len()).then(b.0.cmp(a.0)))
                    .map(|(h, ns)| (h.to_string(), ns.len()));
                if let Some((rewards_hash, count)) = winner {
                    inner.catchup_votes.remove(&epoch);
                    events.push(FinalizedEpoch { epoch, rewards_hash, quorum: count });
                }
            }
            // Bound memory: votes for epochs nobody else is voting on stay small, but
            // never let the map grow without limit.
            if inner.catchup_votes.len() > 200_000 {
                let mut keys: Vec<u64> = inner.catchup_votes.keys().copied().collect();
                keys.sort_unstable();
                for k in keys.into_iter().skip(100_000) {
                    inner.catchup_votes.remove(&k);
                }
            }
        }
        let n = events.len();
        for ev in events {
            let _ = self.finalized_tx.send(ev);
        }
        n
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

        // Device clock seats ("{owner}@{pubkey}") are bound to their key: the seal
        // must carry a signature that verifies against the pubkey in the id itself
        // (the seal's own pubkey field is not trusted for this).
        if let Some((_, dev_pk)) = hone_types::parse_device_clock_id(&parsed.node_id) {
            let ok = parsed.signature.as_deref()
                .map(|sig| verify_seal_signature(&parsed, dev_pk, sig))
                .unwrap_or(false);
            if !ok {
                warn!("[clock] rejected device seal from {} — missing or bad signature", parsed.node_id);
                return;
            }
        }

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
                    quorum_warned: false,
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
        // Abandon epochs that never reached quorum once they are more than 20 epochs
        // behind the newest tracked epoch. Pruning used to run only after a SUCCESSFUL
        // resolution, so a stuck epoch was retried (and logged) every second for up
        // to 20 epochs, and an isolated node (nothing resolving) kept every unresolved
        // epoch forever. Seals that late are already dropped as stale by the gossip
        // handler, so nothing that could still resolve is lost.
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(newest) = inner.epoch_states.keys().max().copied() {
                let mut abandoned: Vec<u64> = inner.epoch_states.iter()
                    .filter(|(e, st)| **e + 20 < newest && !st.resolved)
                    .map(|(e, _)| *e)
                    .collect();
                abandoned.sort_unstable();
                for e in &abandoned {
                    info!("[clock] epoch {}: abandoned unresolved (no quorum within 20 epochs)", e);
                }
                inner.epoch_states.retain(|e, _| *e + 20 >= newest);
            }
        }
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
    /// diverging state between nodes. Seeding an entry here makes `tick()` resolve the
    /// epoch as `sealed:false` on every node instead of ignoring it.
    ///
    /// ⚠️ KNOWN-INCOMPLETE (Grouchly verify pass, 938bdc8c): resolving `sealed:false`
    /// alone is NOT yet full negative attestation — nothing currently consumes the
    /// `sealed:false` branch into a reward proposal for a *globally* empty epoch (the
    /// SealedEpoch{sealed:false} path falls through to a no-op in main.rs), so a truly
    /// empty epoch still does not reach a quorum-agreed FinalizedEpoch on its own. It
    /// only becomes reachable via the peer-ingest proposal path — which is itself the
    /// locally-empty-but-remotely-sealed double-credit race. The correct fix (replay-
    /// derive the mutation off the fork-choice-winning EpochFinalize in apply_entry) is
    /// tracked separately; this comment stays honest until that lands. Idempotent: a
    /// later seal for the epoch simply appends to the existing entry.
    pub fn ensure_epoch_tracked(&self, epoch: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.epoch_states.entry(epoch).or_insert_with(|| EpochState {
            seals: Vec::new(),
            resolved: false,
            winner: None,
            deadline: Instant::now() + Duration::from_millis(SEAL_COLLECT_MS),
            peer_fallback_deadline: None,
            quorum_warned: false,
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

    /// Inject the reputation weights + enable flag that govern the decision for a SPECIFIC
    /// epoch. Called from main.rs when the prior epoch seals (anchoring `epoch`'s decision to
    /// committed-state-through-`epoch − 1`) and at startup for the pending epoch, computed from
    /// on-chain state (role_stake:clock, clock_uptime) via `clock_weight()`. `enabled` is the
    /// chain-param gate — false (the default) → the flat count rule governs and weights are
    /// ignored. Epoch-keyed so the decision for `epoch` is anchor-stable across nodes and restarts.
    pub fn set_clock_weights(&self, epoch: u64, weights: HashMap<String, u128>, enabled: bool) {
        let mut inner = self.inner.lock().unwrap();
        inner.epoch_weights.insert(epoch, (enabled, weights));
        // Bound the map behind the TRUE newest key, not just the epoch we happened to inject —
        // two writers feed this (startup seed + seal handler) and the startup seed can be ahead
        // of the seal handler's first injection, so pruning relative to `epoch` alone could wipe
        // entries the seal path still needs. Compute the max over all keys.
        if let Some(&newest) = inner.epoch_weights.keys().max() {
            let cutoff = newest.saturating_sub(40);
            inner.epoch_weights.retain(|e, _| *e >= cutoff);
        }
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
            // Epoch-anchored weights: the decision for THIS epoch uses weights injected for it
            // (from committed-state-through-(epoch−1)), never a mutable latest — so all nodes,
            // and a node before vs. after a restart, resolve this epoch from the same anchor.
            let (weighted_quorum_enabled, clock_weights) = inner
                .epoch_weights
                .get(&epoch)
                .cloned()
                .unwrap_or((false, HashMap::new()));

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

                // Decide the winning seal_hash and whether quorum is met.
                //
                // Weighted path (v1, gated): when the chain param is ON and the registered set
                // carries nonzero total on-chain weight, an epoch seals only when the clocks
                // signing one seal_hash hold >51% of the (capped) registered weight AND meet the
                // absolute count floor `quorum()`. Weights come purely from on-chain state, so
                // every node reaches the identical decision. When the param is OFF, or every
                // registered clock is still in probation (total weight 0), fall through to the
                // flat >51%-of-count rule below — byte-identical to pre-change behaviour.
                let reg_weight_total: u128 = registered_clocks.iter()
                    .map(|n| clock_weights.get(n).copied().unwrap_or(0))
                    .sum();
                let use_weighted =
                    weighted_quorum_enabled && !registered_clocks.is_empty() && reg_weight_total > 0;

                let winner_hash: String = if use_weighted {
                    let votes: Vec<(String, String)> = voting_inliers.iter()
                        .map(|s| (s.node_id.clone(), s.seal_hash.clone()))
                        .collect();
                    match weighted_winner(&votes, &registered_clocks, &clock_weights, quorum()) {
                        Some((hash, count, weight)) => {
                            info!(
                                "[clock] epoch {}: WEIGHTED quorum engaged — winner weight {} / {} registered ({} signers, floor {})",
                                epoch, weight, reg_weight_total, count, quorum()
                            );
                            hash
                        }
                        None => {
                            if inner.first_quorum_warn(epoch) {
                            warn!(
                                "[clock] epoch {}: insufficient WEIGHTED quorum ({} registered inliers / {} registered, floor {}; {} total seals received)",
                                epoch, voting_inliers.len(), denominator, quorum(), seals.len()
                            );
                        } else {
                            tracing::debug!(
                                "[clock] epoch {}: insufficient WEIGHTED quorum ({} registered inliers / {} registered, floor {}; {} total seals received)",
                                epoch, voting_inliers.len(), denominator, quorum(), seals.len()
                            );
                        }
                            return;
                        }
                    }
                } else {
                    let quorum_needed = std::cmp::max(
                        1,
                        (denominator as f64 * MIN_QUORUM_FRACTION).ceil() as usize,
                    );
                    if voting_inliers.len() < quorum_needed {
                        if inner.first_quorum_warn(epoch) {
                            warn!(
                            "[clock] epoch {}: insufficient quorum ({} registered inliers / {} registered, need {}; {} total seals received)",
                            epoch, voting_inliers.len(), denominator, quorum_needed, seals.len()
                        );
                        } else {
                            tracing::debug!(
                            "[clock] epoch {}: insufficient quorum ({} registered inliers / {} registered, need {}; {} total seals received)",
                            epoch, voting_inliers.len(), denominator, quorum_needed, seals.len()
                        );
                        }
                        return;
                    }
                    // Among voting inliers, pick the winning seal_hash (most common).
                    let mut hash_count: HashMap<&str, usize> = HashMap::new();
                    for s in &voting_inliers {
                        *hash_count.entry(s.seal_hash.as_str()).or_default() += 1;
                    }
                    hash_count.iter().max_by_key(|(_, c)| *c)
                        .map(|(h, _)| h.to_string()).unwrap_or_default()
                };

                let winner_seals: Vec<EpochSeal> = voting_inliers.iter()
                    .filter(|s| s.seal_hash == winner_hash).cloned().collect();

                for s in &winner_seals {
                    inner.update_clock_score(&s.node_id, true);
                }
                for s in voting_inliers.iter().filter(|s| s.seal_hash != winner_hash) {
                    inner.update_clock_score(&s.node_id, false);
                }
                // Timestamp outliers were ALREADY scored down at the top of this block (where the
                // outlier warning is logged). The duplicate loop that used to be here scored the
                // same set a second time — a −20 / +2 double-rate decay per epoch, not −10 / +1
                // (Beastly review 940a6577). Harmless while scores are API-only, but a consensus
                // bug the moment scores feed quorum weight, and it already inflated the
                // outlier_count reported by get_scores(). Removed — score outliers exactly once.

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
/// The reward-consensus VOTE key for an epoch.
///
/// BUG 6 (vote nondeterminism, Grouchly 1f963ba9 → spec dbaa3cdf / e076cd29): this used to
/// hash seven WORK-ENTRY prefixes (mine/storage_beat/sensor_commit/tracker_sighting/
/// infer_verify/service_beat/mempool_beat) via a live local store scan. Every one of those
/// is applied-locally-then-gossiped, so two honest nodes computed DIFFERENT hashes for the
/// same epoch depending on which peers' entries had propagated at scan time. The tally
/// buckets by hash (`hash_counts`), so different hashes = vote-SPLITTING = neither reaches
/// quorum = the epoch never finalizes. Reward consensus "fired once" only when the input
/// happened to be empty on both sides (sha256 of nothing) by coincidence.
///
/// Crucially the vote hash is VESTIGIAL: `emit_epoch_rewards` derives the actual payout
/// from `sealed_by`, NOT from this hash — so the nodes were failing to agree on a value
/// that doesn't drive the payout. The replay-derivation fix (e86a03730) made APPLICATION
/// deterministic via `sealed_by`; this makes the VOTE deterministic over the SAME data.
///
/// We now hash the `epoch_validators:{epoch}` snapshot — the registered clock set, written
/// (main.rs) BEFORE this is called, identical on every node (it comes from
/// `registered_clock_nodes`, sorted + deduped, not from gossip-timed entries), and it is
/// the LITERAL key that becomes the winning EpochFinalize's `sealed_by` (main.rs read).
/// So the vote input and the application input are the same bytes: nodes agree per-epoch,
/// and agreement is over exactly the data that determines rewards.
/// Operator of a clock id: the account before '@' for a device seat ("josh@<pk>" → "josh"),
/// else the id itself.
pub fn clock_operator(id: &str) -> &str {
    id.split_once('@').map(|(o, _)| o).unwrap_or(id)
}

/// Registered clocks that vote on rewards: full nodes only. Device seats seal time but
/// never compute a rewards_hash, so they are not in the reward quorum.
pub fn reward_voters(reg: &[String]) -> Vec<String> {
    reg.iter().filter(|n| hone_types::parse_device_clock_id(n).is_none()).cloned().collect()
}

/// Minimum distinct operators that must agree before rewards finalize. No single
/// operator can pay itself: a thousand "josh" nodes still need a natoshisakamoto or
/// shindevlin node to agree. HONE_MIN_REWARD_OPERATORS overrides (isolated test rigs
/// only — must be identical on every node or it is a fork vector).
pub fn min_reward_operators() -> usize {
    std::env::var("HONE_MIN_REWARD_OPERATORS").ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2)
}

/// True when the proposals backing `hash` come from at least `min_reward_operators()`
/// distinct operators.
fn operators_agree(proposals: &[&RewardProposal], hash: &str) -> bool {
    let ops: HashSet<&str> = proposals.iter()
        .filter(|p| p.rewards_hash == hash)
        .map(|p| clock_operator(&p.node_id))
        .collect();
    ops.len() >= min_reward_operators()
}

pub fn compute_rewards_hash(epoch: u64, store: &Store) -> String {
    let snap = store.state_get(&format!("epoch_validators:{}", epoch));
    rewards_hash_over(epoch, snap.as_deref().unwrap_or(&[]))
}

fn rewards_hash_over(epoch: u64, snapshot: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"epoch:");
    hasher.update(epoch.to_le_bytes());
    hasher.update(b"|validators:");
    // Hash the raw stored snapshot bytes directly — deterministic across nodes because
    // the Vec<String> was sorted+deduped before serialization. Absent snapshot hashes as
    // empty (see `no_reward_hash`).
    hasher.update(snapshot);
    format!("{:x}", hasher.finalize())
}

/// The canonical "this epoch earned nothing" rewards_hash: the hash over NO validator
/// snapshot. It is a pure function of the epoch number, so every node computes the same
/// bytes whatever its local store holds. A real snapshot is a JSON array (at least "[]"),
/// so no live seal-time hash can ever collide with it.
///
/// Catch-up consensus votes this hash (main.rs, catch-up sender; the only exception is
/// co-signing a peer's echoed live finalize over our own snapshot, adoptable_echoes). The
/// `epoch_validators:{e}` snapshot is written by the LOCAL seal handler, including on a
/// solo self-seal, so for an epoch the network never sealed with quorum one node holds a
/// snapshot and another does not. Voting `compute_rewards_hash` there made the two nodes'
/// votes differ forever (mainnet 2026-10-31..11-02: 8,490 epochs from 339222 stuck, the
/// reward driver blocked on 339222). An epoch that missed live reward consensus earns
/// nothing; its block reward recycles, identically on every node.
pub fn no_reward_hash(epoch: u64) -> String {
    rewards_hash_over(epoch, &[])
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
        // Device seats: ignored before activation; afterwards only the owner's own
        // stake counts toward the seat's minimum.
        if let Some((owner, _)) = hone_types::parse_device_clock_id(&node) {
            if current_epoch < hone_types::DEVICE_CLOCK_ACTIVATION_EPOCH { continue; }
            if &rest[sep + 1..] != owner { continue; }
        }
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

/// Compute the reputation weight for every registered clock from ON-CHAIN state only —
/// summed `role_stake:clock:{node}:*` and the `clock_uptime:{node}` record. A pure function
/// of the committed store snapshot: deterministic and restart-invariant (spec §9). Feeds
/// `weighted_winner` via `ClockConsensus::set_clock_weights`.
pub fn compute_clock_weights(store: &Store, registered: &[String]) -> HashMap<String, u128> {
    // Aggregate self + backer stake per node (same key layout as registered_clock_nodes).
    let mut stake: HashMap<String, u64> = HashMap::new();
    for (key, val) in store.state_scan_prefix("role_stake:clock:") {
        let Some(rest) = key.strip_prefix("role_stake:clock:") else { continue };
        let Some(sep) = rest.rfind(':') else { continue };
        let node = rest[..sep].to_owned();
        let j: serde_json::Value = match serde_json::from_slice(&val) {
            Ok(v) => v,
            Err(_) => continue,
        };
        *stake.entry(node).or_insert(0) += j["amount"].as_u64().unwrap_or(0);
    }

    let mut out = HashMap::new();
    for node in registered {
        let stake_hunits = stake.get(node).copied().unwrap_or(0);
        // REAL attendance, not registration: read `device_attend:{node}` ({sealed, epochs}),
        // accrued in the ordered reward walk from the actual per-device seal record — NOT
        // `clock_uptime` (which is fed by the registered set and reads ~100% for everyone, so
        // it discriminates nothing). A device dark for a stretch has a low sealed/epochs ratio
        // here → low uptime_term → sinking weight. See emit_epoch_rewards + §0 of the spec.
        let att: serde_json::Value = store
            .state_get(&format!("device_attend:{}", node))
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(serde_json::json!({ "sealed": 0, "epochs": 0 }));
        let sealed = att["sealed"].as_u64().unwrap_or(0);
        let epochs = att["epochs"].as_u64().unwrap_or(0);
        out.insert(node.clone(), clock_weight(sealed, epochs, stake_hunits));
    }
    out
}

/// Activation epoch for reputation-weighted quorum. Chain param `chain_param:weighted_quorum`
/// holds the epoch AT WHICH the feature turns on (0 / absent = never = OFF). It is an EPOCH,
/// not a bool, on purpose: the attendance accrual and the weighted decision are anchored to the
/// epoch being processed (`epoch >= activation`), NOT to "is the flag set right now". Reading a
/// bool "now" would retroactively change how already-settled epochs accrue — the walk runs ~20
/// epochs behind the tip, so "now" is always past activation and every node would fold in a
/// different pre-activation tail. Set via a tx-applied `ChainParameterSet` (agreed at a height),
/// with the activation epoch chosen comfortably in the future so every node holds the param
/// before its walk reaches it. Default OFF — ships dormant until deliberately activated
/// (branch-only, gate-then-Shin-in-person).
pub fn weighted_quorum_activation_epoch(store: &Store) -> u64 {
    store
        .state_get("chain_param:weighted_quorum")
        .and_then(|b| serde_json::from_slice::<u64>(&b).ok())
        .unwrap_or(0)
}

/// Whether weighted quorum governs the decision/accrual for a SPECIFIC epoch: activated AND the
/// epoch is at or past the activation height. Epoch-anchored so it is a pure function of agreed
/// state, identical on every node and across restarts.
pub fn weighted_quorum_active_for(store: &Store, epoch: u64) -> bool {
    let act = weighted_quorum_activation_epoch(store);
    act != 0 && epoch >= act
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
mod weighted_quorum_tests {
    use super::*;

    fn w(pairs: &[(&str, u128)]) -> HashMap<String, u128> {
        pairs.iter().map(|(n, v)| (n.to_string(), *v)).collect()
    }
    fn votes(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(n, h)| (n.to_string(), h.to_string())).collect()
    }
    fn reg(ns: &[&str]) -> Vec<String> {
        ns.iter().map(|s| s.to_string()).collect()
    }

    // ── isqrt: bit-exact, the cross-arch determinism primitive ──────────────────
    #[test]
    fn isqrt_is_exact() {
        assert_eq!(isqrt_u128(0), 0);
        assert_eq!(isqrt_u128(1), 1);
        assert_eq!(isqrt_u128(3), 1);
        assert_eq!(isqrt_u128(4), 2);
        assert_eq!(isqrt_u128(8), 2);
        assert_eq!(isqrt_u128(9), 3);
        assert_eq!(isqrt_u128(15), 3);
        assert_eq!(isqrt_u128(16), 4);
        assert_eq!(isqrt_u128(1_000_000), 1000);
        // floor property holds across a sweep — no float, no off-by-one at any boundary
        for n in 0u128..=100_000 {
            let r = isqrt_u128(n);
            assert!(r * r <= n && (r + 1) * (r + 1) > n, "isqrt({n}) = {r}");
        }
        // large value near u128 range still satisfies the floor property
        let big = 1u128 << 100;
        let r = isqrt_u128(big);
        assert!(r * r <= big && (r + 1) * (r + 1) > big);
    }

    // ── probation (§3): a fresh clock carries no weight ─────────────────────────
    #[test]
    fn probation_new_clock_has_zero_weight() {
        // epochs below the probation floor → 0 regardless of seals/stake
        assert_eq!(clock_weight(9, 9, 1_000_000_000_000), 0);
        // just past probation → nonzero
        assert!(clock_weight(10, 10, 0) > 0);
    }

    // ── diminishing returns (§4): stake_term grows as isqrt, not linearly ───────
    #[test]
    fn stake_term_diminishes() {
        // 100x the stake must NOT yield ~100x the weight (isqrt → ~10x on the stake part)
        let base = clock_weight(100, 100, 10_000_000_000); // 1 HONE
        let big = clock_weight(100, 100, 1_000_000_000_000); // 100 HONE
        assert!(big > base);
        assert!(big < base * 100, "stake must have diminishing returns, got {big} vs {base}");
    }

    // ── NO SPLIT-BRAIN (§9): two disjoint sets can't each clear 51% ─────────────
    #[test]
    fn no_split_brain() {
        let registered = reg(&["a", "b", "c", "d"]);
        let weights = w(&[("a", 1000), ("b", 1000), ("c", 1000), ("d", 1000)]);
        // set {a,b} votes hashX, {c,d} votes hashY — a perfect even split
        let split = votes(&[("a", "X"), ("b", "X"), ("c", "Y"), ("d", "Y")]);
        // neither hash exceeds 51% of total weight → NO winner (no fork)
        assert!(weighted_winner(&split, &registered, &weights, 2).is_none());
    }

    // ── NO STALL FROM A LOW-WEIGHT DROP (§9): the whole point ───────────────────
    #[test]
    fn low_weight_clock_dropping_does_not_stall() {
        // three established high-weight clocks + one near-zero laptop
        let registered = reg(&["g", "n", "b", "laptop"]);
        let weights = w(&[("g", 1000), ("n", 1000), ("b", 1000), ("laptop", 1)]);
        // laptop is DOWN (doesn't vote); the three established clocks agree
        let v = votes(&[("g", "X"), ("n", "X"), ("b", "X")]);
        let win = weighted_winner(&v, &registered, &weights, 2).expect("must still seal");
        assert_eq!(win.0, "X");
        // and a lone laptop can NEVER swing or seal on its own
        let solo = votes(&[("laptop", "Y")]);
        assert!(weighted_winner(&solo, &registered, &weights, 2).is_none());
    }

    // ── FLOOR HONORED (§9): weight majority can't drop effective quorum below floor
    #[test]
    fn floor_is_honored() {
        // one clock holds >51% of weight, but the count floor is 2 → its solo vote can't seal
        let registered = reg(&["heavy", "light"]);
        let weights = w(&[("heavy", 1000), ("light", 100)]);
        let solo = votes(&[("heavy", "X")]);
        assert!(
            weighted_winner(&solo, &registered, &weights, 2).is_none(),
            "solo vote must fail the count floor even with majority weight"
        );
    }

    // ── ANTI-CONCENTRATION CAP (§6): no single clock exceeds 1/3, pre-cap, no renorm
    #[test]
    fn one_clock_capped_at_one_third() {
        // a whale with 97% of raw weight is capped to 1/3 — its solo agreement with one
        // small clock still can't clear 51% of the (capped) total on its own count-of-2…
        let registered = reg(&["whale", "a", "b"]);
        let weights = w(&[("whale", 970), ("a", 15), ("b", 15)]);
        // whale alone (capped to 1/3 ≈ 333 of raw 1000) can't be >51% of capped total
        let whale_solo = votes(&[("whale", "X"), ("a", "X")]); // needs count>=2, borrows a
        let win = weighted_winner(&whale_solo, &registered, &weights, 2);
        // capped whale (333) + a (15) = 348; capped total = 333+15+15 = 363; 348/363 > 51% → seals,
        // but ONLY because a genuine second clock agreed — the whale can't do it alone:
        assert!(win.is_some());
        let whale_only = votes(&[("whale", "X")]);
        assert!(weighted_winner(&whale_only, &registered, &weights, 2).is_none());
    }

    // ── DETERMINISM / RESTART-INVARIANCE (§9, THE key assertion) ────────────────
    // Weight is a pure function of on-chain (seals, epochs, stake). Same inputs →
    // byte-identical output, every call, every process, every architecture. A restart
    // changes no input, so it changes no weight. Proven by construction: recomputing
    // yields the identical value with zero intervening state.
    #[test]
    fn weight_is_pure_and_restart_invariant() {
        let before = clock_weight(87, 100, 42_000_000_000);
        // …simulate a process restart: nothing about the on-chain inputs changes…
        let after = clock_weight(87, 100, 42_000_000_000);
        assert_eq!(before, after);
        // and the winner selection is deterministic under identical inputs
        let registered = reg(&["a", "b", "c"]);
        let weights = w(&[("a", 500), ("b", 400), ("c", 300)]);
        let v = votes(&[("a", "X"), ("b", "X"), ("c", "X")]);
        let r1 = weighted_winner(&v, &registered, &weights, 2);
        let r2 = weighted_winner(&v, &registered, &weights, 2);
        assert_eq!(r1, r2);
    }

    // ── FALLBACK: all-probation cohort returns None so caller uses flat count ────
    #[test]
    fn all_zero_weight_falls_back_to_flat() {
        let registered = reg(&["a", "b"]);
        let weights = w(&[("a", 0), ("b", 0)]);
        let v = votes(&[("a", "X"), ("b", "X")]);
        // total weight 0 → None → caller applies the flat count rule (never stalls genesis)
        assert!(weighted_winner(&v, &registered, &weights, 2).is_none());
    }

    // ── gate wiring end-to-end through the store ────────────────────────────────
    #[test]
    fn weights_and_gate_read_from_store() {
        let dir = tempfile::Builder::new().prefix("hone_wq_").tempdir().unwrap();
        let s = crate::store::Store::open(dir.path()).unwrap();

        // gate defaults OFF when the chain param is unset
        assert!(!weighted_quorum_active_for(&s, 1_000_000));
        // activation epoch is an EPOCH, not a bool: active only for epochs >= activation
        s.state_set("chain_param:weighted_quorum", &serde_json::to_vec(&500u64).unwrap()).unwrap();
        assert!(!weighted_quorum_active_for(&s, 499), "must be OFF before the activation epoch");
        assert!(weighted_quorum_active_for(&s, 500), "must be ON at the activation epoch");
        assert!(weighted_quorum_active_for(&s, 5_000), "must stay ON after activation");
        // 0 = never
        s.state_set("chain_param:weighted_quorum", &serde_json::to_vec(&0u64).unwrap()).unwrap();
        assert!(!weighted_quorum_active_for(&s, 5_000));

        // established clock: 10 HONE self-stake, 95/100 REAL attendance
        s.state_set(
            "role_stake:clock:estab:estab",
            &serde_json::to_vec(&serde_json::json!({"amount": 100_000_000_000u64, "staked_epoch": 1}))
                .unwrap(),
        ).unwrap();
        s.state_set(
            "device_attend:estab",
            &serde_json::to_vec(&serde_json::json!({"sealed": 95, "epochs": 100})).unwrap(),
        ).unwrap();
        // fresh clock: staked but only 3 epochs of attendance → still in probation
        s.state_set(
            "role_stake:clock:fresh:fresh",
            &serde_json::to_vec(&serde_json::json!({"amount": 100_000_000_000u64, "staked_epoch": 90}))
                .unwrap(),
        ).unwrap();
        s.state_set(
            "device_attend:fresh",
            &serde_json::to_vec(&serde_json::json!({"sealed": 3, "epochs": 3})).unwrap(),
        ).unwrap();

        let registered = reg(&["estab", "fresh"]);
        let weights = compute_clock_weights(&s, &registered);
        assert!(weights["estab"] > 0, "established clock must carry weight");
        assert_eq!(weights["fresh"], 0, "probation clock must carry zero weight");

        // with only the established clock holding weight, dropping the fresh one can't stall:
        let v = votes(&[("estab", "X")]);
        // count floor 2 blocks a solo seal even though estab holds 100% of the weight —
        // this is the floor honored jointly with the weighted rule (both from real store data)
        assert!(weighted_winner(&v, &registered, &weights, 2).is_none());
        // but two agreeing established-class clocks would seal (simulate a second real clock)
        let mut w2 = weights.clone();
        w2.insert("estab2".into(), weights["estab"]);
        let registered2 = reg(&["estab", "estab2", "fresh"]);
        let v2 = votes(&[("estab", "X"), ("estab2", "X")]);
        assert!(weighted_winner(&v2, &registered2, &w2, 2).is_some());
    }

    // ── real attendance discriminates: a dark device sinks below a reliable one ──
    // The whole point of the seal-anchored record over registration-fed clock_uptime:
    // two equally-staked, equally-tenured devices get DIFFERENT weight when one actually
    // seals and the other goes dark.
    #[test]
    fn real_attendance_makes_a_dark_device_sink() {
        let dir = tempfile::Builder::new().prefix("hone_attend_").tempdir().unwrap();
        let s = crate::store::Store::open(dir.path()).unwrap();
        // identical stake
        for n in ["reliable", "flaky"] {
            s.state_set(
                &format!("role_stake:clock:{n}:{n}"),
                &serde_json::to_vec(&serde_json::json!({"amount": 100_000_000_000u64, "staked_epoch": 1}))
                    .unwrap(),
            ).unwrap();
        }
        // same window length, very different ACTUAL attendance
        s.state_set("device_attend:reliable",
            &serde_json::to_vec(&serde_json::json!({"sealed": 99, "epochs": 100})).unwrap()).unwrap();
        s.state_set("device_attend:flaky",
            &serde_json::to_vec(&serde_json::json!({"sealed": 20, "epochs": 100})).unwrap()).unwrap();

        let registered = reg(&["reliable", "flaky"]);
        let weights = compute_clock_weights(&s, &registered);
        assert!(
            weights["reliable"] > weights["flaky"] * 3,
            "a device that seals 99/100 must massively outweigh one that seals 20/100 at equal stake — got {} vs {}",
            weights["reliable"], weights["flaky"]
        );
        // and the reliable device alone can't be outvoted by the flaky one under the weighted rule
        let v = votes(&[("reliable", "X"), ("flaky", "Y")]);
        // neither reaches >51% on its own here (2 clocks, split) → no seal, but reliable holds
        // the larger share, so if flaky drops, reliable + any peer carries. Sanity: weights sane.
        assert!(weighted_winner(&v, &registered, &weights, 2).is_none());
    }

    // ── unregistered votes carry no weight ──────────────────────────────────────
    #[test]
    fn unregistered_voter_is_ignored() {
        let registered = reg(&["a", "b", "c"]);
        let weights = w(&[("a", 1000), ("b", 1000), ("c", 1000)]);
        // an outsider "z" (not registered) piles onto hash Y; it must contribute nothing
        let v = votes(&[("a", "X"), ("b", "X"), ("z", "Y"), ("z", "Y")]);
        let win = weighted_winner(&v, &registered, &weights, 2).expect("a+b agree");
        assert_eq!(win.0, "X");
    }
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

#[cfg(test)]
mod device_clock_tests {
    use super::*;

    const PK: &str = "8848e0d2db7804aa1c15e500f7655cffcd84ff1635214ab55c0e2d46f66d4f25";

    fn store() -> (crate::store::Store, tempfile::TempDir) {
        let dir = tempfile::Builder::new().prefix("hone_dev_clock_").tempdir().unwrap();
        let s = crate::store::Store::open(dir.path()).unwrap();
        (s, dir)
    }

    fn stake(s: &crate::store::Store, node: &str, staker: &str, amount: u64) {
        let rec = serde_json::json!({"node": node, "role": "clock", "staker": staker,
            "amount": amount, "staked_epoch": 1});
        s.state_set(&format!("role_stake:clock:{}:{}", node, staker),
            &serde_json::to_vec(&rec).unwrap()).unwrap();
    }

    #[test]
    fn parses_device_ids() {
        let id = format!("josh@{PK}");
        assert_eq!(hone_types::parse_device_clock_id(&id), Some(("josh", PK)));
        assert!(hone_types::parse_device_clock_id("josh").is_none());
        assert!(hone_types::parse_device_clock_id("josh@abcd").is_none());
        assert!(hone_types::parse_device_clock_id(&format!("@{PK}")).is_none());
        assert!(hone_types::parse_device_clock_id(&format!("josh@{}", PK.to_uppercase())).is_none());
        assert!(hone_types::parse_device_clock_id(&format!("a:b@{PK}")).is_none());
    }

    #[test]
    fn device_seat_needs_activation_and_owner_stake() {
        let (s, _d) = store();
        let dev = format!("josh@{PK}");
        let min = 5 * 10_000_000_000u64;
        stake(&s, &dev, "josh", min);
        let before = registered_clock_nodes(&s, hone_types::DEVICE_CLOCK_ACTIVATION_EPOCH - 1);
        assert!(!before.contains(&dev), "device seat must not count before activation");
        let after = registered_clock_nodes(&s, hone_types::DEVICE_CLOCK_ACTIVATION_EPOCH);
        assert!(after.contains(&dev), "owner-staked device seat counts after activation");

        let (s2, _d2) = store();
        stake(&s2, &dev, "mallory", min);
        let other = registered_clock_nodes(&s2, hone_types::DEVICE_CLOCK_ACTIVATION_EPOCH);
        assert!(!other.contains(&dev), "only the owner's stake counts for a device seat");
    }

    #[test]
    fn two_devices_of_one_account_are_two_seats() {
        let (s, _d) = store();
        let min = 5 * 10_000_000_000u64;
        let pk2 = "11".repeat(32);
        let d1 = format!("josh@{PK}");
        let d2 = format!("josh@{pk2}");
        stake(&s, &d1, "josh", min);
        stake(&s, &d2, "josh", min);
        let nodes = registered_clock_nodes(&s, hone_types::DEVICE_CLOCK_ACTIVATION_EPOCH);
        assert!(nodes.contains(&d1) && nodes.contains(&d2));
    }

    fn seal(node: &str, epoch: u64, sk: Option<&ed25519_dalek::SigningKey>) -> serde_json::Value {
        use ed25519_dalek::Signer;
        let ts = 1_000u64;
        let hash = "h".to_string();
        let sig = sk.map(|k| hex::encode(k.sign(
            format!("seal:{}:{}:{}:{}", epoch, hash, node, ts).as_bytes()).to_bytes()));
        serde_json::json!({"epoch_number": epoch, "node_id": node, "timestamp": ts,
            "seal_hash": hash, "signature": sig,
            "pubkey": sk.map(|k| hex::encode(k.verifying_key().to_bytes()))})
    }

    fn seals_for(c: &ClockConsensus, epoch: u64) -> usize {
        c.inner.lock().unwrap().epoch_states.get(&epoch).map(|s| s.seals.len()).unwrap_or(0)
    }

    #[test]
    fn device_seal_must_be_signed_by_the_key_in_its_id() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let other = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let pk = hex::encode(sk.verifying_key().to_bytes());
        let dev = format!("josh@{pk}");
        let c = ClockConsensus::new();
        c.receive_seal(seal(&dev, 10, None));
        assert_eq!(seals_for(&c, 10), 0, "unsigned device seal rejected");
        c.receive_seal(seal(&dev, 11, Some(&other)));
        assert_eq!(seals_for(&c, 11), 0, "device seal signed by another key rejected");
        c.receive_seal(seal(&dev, 12, Some(&sk)));
        assert_eq!(seals_for(&c, 12), 1, "correctly signed device seal accepted");
    }

    #[test]
    fn stale_unresolved_epochs_are_abandoned() {
        let c = ClockConsensus::new();
        c.ensure_epoch_tracked(100);
        c.ensure_epoch_tracked(121);
        c.tick();
        let inner = c.inner.lock().unwrap();
        assert!(!inner.epoch_states.contains_key(&100), "21 behind: abandoned");
        assert!(inner.epoch_states.contains_key(&121));
    }
}

#[cfg(test)]
mod reward_quorum_tests {
    use super::*;

    const PHONE: &str = "josh@8848e0d2db7804aa1c15e500f7655cffcd84ff1635214ab55c0e2d46f66d4f25";

    fn reg() -> Vec<String> {
        vec!["josh".into(), PHONE.into(), "natoshisakamoto".into(), "shindevlin".into()]
    }

    fn propose(c: &ClockConsensus, epoch: u64, node: &str, hash: &str) {
        c.receive_reward_proposal(RewardProposal {
            epoch, node_id: node.into(), rewards_hash: hash.into(),
        });
    }

    #[test]
    fn device_seats_do_not_vote_on_rewards() {
        assert_eq!(reward_voters(&reg()), vec!["josh", "natoshisakamoto", "shindevlin"]);
        assert_eq!(clock_operator(PHONE), "josh");
    }

    #[test]
    fn two_servers_of_two_operators_finalize_with_phone_registered() {
        let c = ClockConsensus::new();
        c.set_registered_clocks(reg());
        let mut rx = c.subscribe_finalized();
        propose(&c, 5, "josh", "h");
        assert!(rx.try_recv().is_err(), "one vote is not quorum");
        propose(&c, 5, "natoshisakamoto", "h");
        assert_eq!(rx.try_recv().unwrap().epoch, 5, "2 of 3 voters, 2 operators");
    }

    #[test]
    fn devices_never_count_toward_reward_quorum() {
        let c = ClockConsensus::new();
        c.set_registered_clocks(reg());
        let mut rx = c.subscribe_finalized();
        propose(&c, 6, "josh", "h");
        propose(&c, 6, PHONE, "h"); // a device seat is not a reward voter
        assert!(rx.try_recv().is_err());
        propose(&c, 6, "shindevlin", "h");
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn catchup_finalizes_past_epoch_on_quorum_of_its_own_voters() {
        let c = ClockConsensus::new();
        let mut rx = c.subscribe_finalized();
        let voters = |_e: u64| vec!["josh".to_string(), "natoshisakamoto".to_string()];
        let not_final = |_e: u64| false;
        let n = c.receive_catchup_votes("josh", &[(74477, "h".into())], voters, not_final);
        assert_eq!(n, 0);
        let n = c.receive_catchup_votes("mallory", &[(74477, "h".into())], voters, not_final);
        assert_eq!(n, 0, "non-voter ignored");
        let n = c.receive_catchup_votes("natoshisakamoto", &[(74477, "x".into())], voters, not_final);
        assert_eq!(n, 0, "disagreeing hash is not quorum");
        let n = c.receive_catchup_votes("natoshisakamoto", &[(74477, "h".into())], voters, not_final);
        assert_eq!(n, 1);
        let ev = rx.try_recv().unwrap();
        assert_eq!((ev.epoch, ev.rewards_hash.as_str(), ev.quorum), (74477, "h", 2));
    }

    #[test]
    fn catchup_needs_two_operators() {
        let c = ClockConsensus::new();
        let voters = |_e: u64| vec!["josh".to_string(), "josh2".to_string()];
        // Two distinct non-device ids are two operators by id; the operator rule is
        // about device seats folding into their owner.
        assert_eq!(c.receive_catchup_votes("josh", &[(1, "h".into())], voters, |_| false), 0);
        assert_eq!(c.receive_catchup_votes("josh2", &[(1, "h".into())], voters, |_| false), 1);
        let dev_voters = |_e: u64| vec!["josh".to_string(), PHONE.to_string()];
        let c2 = ClockConsensus::new();
        assert_eq!(c2.receive_catchup_votes("josh", &[(1, "h".into())], dev_voters, |_| false), 0);
        assert_eq!(c2.receive_catchup_votes(PHONE, &[(1, "h".into())], dev_voters, |_| false), 0,
            "same operator twice is not enough");
    }

    #[test]
    fn catchup_skips_already_finalized() {
        let c = ClockConsensus::new();
        let voters = |_e: u64| vec!["a".to_string(), "b".to_string()];
        c.receive_catchup_votes("a", &[(9, "h".into())], voters, |_| true);
        assert_eq!(c.receive_catchup_votes("b", &[(9, "h".into())], voters, |_| true), 0);
    }
}

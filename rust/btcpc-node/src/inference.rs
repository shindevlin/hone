//! Inference job marketplace — bid/accept/verify/dispute state machine.
//!
//! # Job lifecycle (happy path)
//!   Posted → Awarded → Completed → Verified → Paid
//!
//! # Dispute path
//!   Completed → Disputed (verifier rejects) → Claimed (worker contests within window)
//!     → Reviewed (≥ MIN_REVIEW_VOTES human reviewers vote) → Paid or Rejected
//!
//!   If no Claim is filed within CLAIM_WINDOW_EPOCHS, the job enters NoFee:
//!   verifiers still receive their share, but the worker receives nothing.
//!   Requester gets refund of (max_fee − verifier_pool − recycle).
//!
//! # Storage keys (CF_META)
//!   "infer_job:{job_id}"          → JobState JSON
//!   "infer_bid:{job_id}:{bidder}" → BidState JSON
//!   "node_rep:{node_id}"          → NodeReputation JSON
//!   "infer_votes:{job_id}"        → Vec<ReviewVote> JSON
#![allow(dead_code)]

#![allow(dead_code)]
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use btcpc_types::{
    LedgerEntry, NATIVE_TOKEN,
    INFERENCE_FEE_WORKER_BPS, INFERENCE_FEE_VERIFIER_BPS, INFERENCE_FEE_RECYCLE_BPS,
    INFERENCE_FEE_WORKER_DISPUTED_BPS, INFERENCE_FEE_VERIFIER_DISPUTED_BPS,
    INFERENCE_FEE_REVIEWER_BPS, INFERENCE_FEE_RECYCLE_DISPUTED_BPS,
    CLAIM_WINDOW_EPOCHS, MIN_REVIEW_VOTES,
    RECYCLE_FUND_ACCOUNT,
    INFERENCE_PRUNE_WINDOW_EPOCHS,
    VERIFIER_ASSIGNMENT_ENABLED, VERIFIER_THRESHOLD_BYTE,
    VERIFIER_APPROVAL_WINDOW_EPOCHS, VERIFIER_RUBBER_STAMP_BPS, VERIFIER_SUSPEND_BPS, VERIFIER_SUSPEND_EPOCHS,
    VERIFIER_ACTIVE_THRESH_3, VERIFIER_ACTIVE_THRESH_5,
    VERIFIER_VOTE_WEIGHT_NORMAL, VERIFIER_VOTE_WEIGHT_REVIEW,
    INFERENCE_COMMIT_REVEAL_ENABLED,
};

use crate::chain::Chain;

// ── State types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Posted,
    Awarded,
    Completed,
    Verified,
    Disputed,
    Claimed,
    Reviewed,
    Paid,
    Rejected,   // dispute resolved against worker
    NoFee,      // dispute, no claim filed in time
    Cancelled,
    Expired,
}

fn default_min_verifiers() -> u64 { 1 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobState {
    pub job_id: String,
    pub requester: String,
    pub model: String,
    pub mode: String,
    pub input_hash: String,
    pub max_fee: u64,
    pub min_reputation: u64,
    pub bid_window_epochs: u64,
    pub deadline_epoch: u64,
    pub posted_epoch: u64,
    pub status: JobStatus,
    pub winner: Option<String>,
    pub awarded_fee: Option<u64>,
    pub verifiers: Vec<String>,
    pub result_hash: Option<String>,
    pub latency_ms: Option<u64>,
    /// Epoch when a dispute was filed (for claim window tracking).
    pub disputed_epoch: Option<u64>,
    /// Epoch when a claim was filed.
    pub claimed_epoch: Option<u64>,
    /// Epoch when the job was settled and payment was calculated.
    #[serde(default)]
    pub settled_epoch: Option<u64>,
    /// Epoch when the worker submitted their result (for vote timeout tracking).
    #[serde(default)]
    pub completed_epoch: Option<u64>,
    /// If true, proof payload is pinned to btcpc-fs permanently (paid opt-in, D1).
    #[serde(default)]
    pub persist_on_fs: bool,
    /// Board verdicts accumulated before quorum: (verifier_account, verdict_string).
    #[serde(default)]
    pub verdicts: Vec<(String, String)>,
    /// Commit-reveal phase 1: (verifier_account, commit_hash) pairs (T4-2).
    /// When INFERENCE_COMMIT_REVEAL_ENABLED, reveals (InferenceJobVerify) are only
    /// accepted after a matching commit exists and the commit phase is closed.
    #[serde(default)]
    pub commits: Vec<(String, String)>,
    /// Required verifier board size before resolution. Set at post time.
    /// Auto-scales with network size; requester may request more for critical work.
    #[serde(default = "default_min_verifiers")]
    pub min_verifiers: u64,
    /// Machine fingerprint of the node that posted this job. Set server-side at post time.
    /// Bids from the same machine are rejected to prevent self-dealing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BidState {
    pub job_id: String,
    pub bidder: String,
    pub fee: u64,
    pub role: String,
    pub epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewVote {
    pub reviewer: String,
    pub approved: bool,
    pub epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeReputation {
    pub node_id: String,
    pub jobs_accepted: u64,
    pub jobs_completed: u64,
    pub jobs_failed: u64,
    pub total_latency_ms: u64,
    /// Score 0-10000; new nodes start at 5000.
    pub score: u64,
}

impl NodeReputation {
    pub fn new(node_id: &str) -> Self {
        Self {
            node_id: node_id.to_owned(),
            jobs_accepted: 0,
            jobs_completed: 0,
            jobs_failed: 0,
            total_latency_ms: 0,
            score: 5000,
        }
    }

    pub fn recompute_score(&mut self) {
        if self.jobs_accepted == 0 {
            self.score = 5000;
            return;
        }
        let completion = self.jobs_completed * 100 / self.jobs_accepted;
        let avg_latency_ms = if self.jobs_completed == 0 {
            5000
        } else {
            self.total_latency_ms / self.jobs_completed
        };
        let latency_factor = (5000 / avg_latency_ms.max(50)).min(100);
        self.score = (completion * (50 + latency_factor)).min(10000);
    }
}

// ── Verifier board helpers ────────────────────────────────────────────────────

/// Auto-scale required verifier count from active network size.
/// Reads `active_verifier_count` written each epoch seal by main.rs.
pub fn auto_scale_min_verifiers(chain: &Chain) -> u64 {
    let active = chain.store.state_get("active_verifier_count")
        .and_then(|b| <[u8; 8]>::try_from(b.as_slice()).ok())
        .map(u64::from_le_bytes)
        .unwrap_or(0);
    if active >= VERIFIER_ACTIVE_THRESH_5 { 5 }
    else if active >= VERIFIER_ACTIVE_THRESH_3 { 3 }
    else { 1 }
}

/// Resolve board quorum from accumulated verdicts.
/// Returns the winning verdict once `min_verifiers` have voted, or None if still waiting.
/// review_required carries +1 weight so it wins all three-way ties.
fn resolve_quorum_verdict(verdicts: &[(String, String)], min_verifiers: u64) -> Option<&'static str> {
    let voted = verdicts.len() as u64;

    let mut approved_w: u64 = 0;
    let mut rejected_w: u64 = 0;
    let mut review_w: u64 = 0;
    for (_, v) in verdicts {
        match v.as_str() {
            "approved"         => approved_w += VERIFIER_VOTE_WEIGHT_NORMAL,
            "rejected"         => rejected_w += VERIFIER_VOTE_WEIGHT_NORMAL,
            "review_required"  => review_w   += VERIFIER_VOTE_WEIGHT_REVIEW,
            _ => {}
        }
    }

    // Early resolution: check if any verdict already has an unbeatable lead.
    // Remaining votes can each contribute at most VERIFIER_VOTE_WEIGHT_REVIEW (the highest weight).
    let remaining = min_verifiers.saturating_sub(voted);
    let max_swing = remaining * VERIFIER_VOTE_WEIGHT_REVIEW;

    if approved_w > rejected_w.saturating_add(max_swing) && approved_w > review_w.saturating_add(max_swing) {
        return Some("approved");
    }
    if rejected_w > approved_w.saturating_add(max_swing) && rejected_w > review_w.saturating_add(max_swing) {
        return Some("rejected");
    }
    if review_w > approved_w.saturating_add(max_swing) && review_w > rejected_w.saturating_add(max_swing) {
        return Some("review_required");
    }

    // Not enough votes yet unless all min_verifiers have voted.
    if voted < min_verifiers { return None; }

    // All min_verifiers voted, no strict majority — highest weight wins.
    // review_required's +1 ensures it beats any exact tie.
    if review_w >= approved_w && review_w >= rejected_w { Some("review_required") }
    else if approved_w >= rejected_w { Some("approved") }
    else { Some("rejected") }
}

// ── Storage helpers ───────────────────────────────────────────────────────────

fn job_key(id: &str) -> String { format!("infer_job:{}", id) }
fn bid_key(job_id: &str, bidder: &str) -> String { format!("infer_bid:{}:{}", job_id, bidder) }
fn rep_key(node_id: &str) -> String { format!("node_rep:{}", node_id) }
fn votes_key(job_id: &str) -> String { format!("infer_votes:{}", job_id) }

pub fn get_job(chain: &Chain, job_id: &str) -> Option<JobState> {
    chain.store.state_get(&job_key(job_id))
        .and_then(|b| serde_json::from_slice(&b).ok())
}

pub fn set_job(chain: &Chain, state: &JobState) -> Result<()> {
    chain.store.state_set(&job_key(&state.job_id), &serde_json::to_vec(state)?)
}

pub fn get_bids(chain: &Chain, job_id: &str) -> Vec<BidState> {
    let prefix = format!("infer_bid:{}:", job_id);
    chain.store.state_scan_prefix(&prefix)
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice(&v).ok())
        .collect()
}

pub fn set_bid(chain: &Chain, bid: &BidState) -> Result<()> {
    chain.store.state_set(&bid_key(&bid.job_id, &bid.bidder), &serde_json::to_vec(bid)?)
}

pub fn get_reputation(chain: &Chain, node_id: &str) -> NodeReputation {
    chain.store.state_get(&rep_key(node_id))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| NodeReputation::new(node_id))
}

pub fn set_reputation(chain: &Chain, rep: &NodeReputation) -> Result<()> {
    chain.store.state_set(&rep_key(&rep.node_id), &serde_json::to_vec(rep)?)
}

pub fn get_votes(chain: &Chain, job_id: &str) -> Vec<ReviewVote> {
    chain.store.state_get(&votes_key(job_id))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn set_votes(chain: &Chain, job_id: &str, votes: &[ReviewVote]) -> Result<()> {
    chain.store.state_set(&votes_key(job_id), &serde_json::to_vec(votes)?)
}

/// Scan open Posted jobs whose bid window has closed.
pub fn jobs_ready_to_award(chain: &Chain, current_epoch: u64) -> Vec<JobState> {
    chain.store.state_scan_prefix("infer_job:")
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<JobState>(&v).ok())
        .filter(|j| {
            j.status == JobStatus::Posted
                && current_epoch >= j.posted_epoch + j.bid_window_epochs
        })
        .collect()
}

/// Scan Disputed jobs where the claim window has expired with no claim filed.
pub fn jobs_claim_expired(chain: &Chain, current_epoch: u64) -> Vec<JobState> {
    chain.store.state_scan_prefix("infer_job:")
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<JobState>(&v).ok())
        .filter(|j| {
            j.status == JobStatus::Disputed
                && j.disputed_epoch
                    .map(|de| current_epoch > de + CLAIM_WINDOW_EPOCHS)
                    .unwrap_or(false)
        })
        .collect()
}

/// Scan jobs past deadline that are still Posted or Awarded.
pub fn jobs_past_deadline(chain: &Chain, current_epoch: u64) -> Vec<JobState> {
    chain.store.state_scan_prefix("infer_job:")
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<JobState>(&v).ok())
        .filter(|j| {
            matches!(j.status, JobStatus::Posted | JobStatus::Awarded)
                && current_epoch > j.deadline_epoch
        })
        .collect()
}

// ── Entry application ─────────────────────────────────────────────────────────

pub fn apply_post(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::InferenceJobPost {
        job_id, requester, model, mode, input_hash,
        max_fee, min_reputation, bid_window_epochs, deadline_epoch, epoch,
        persist_on_fs, min_verifiers: requested_min_verifiers, node_fingerprint, ..
    } = entry else { bail!("wrong entry type") };

    if get_job(chain, job_id).is_some() {
        bail!("job '{}' already exists", job_id);
    }
    chain.store.debit(requester, NATIVE_TOKEN, *max_fee)?;
    chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, *max_fee)?;

    let auto_min = auto_scale_min_verifiers(chain);
    let min_verifiers = auto_min.max(requested_min_verifiers.unwrap_or(0));

    set_job(chain, &JobState {
        job_id: job_id.clone(),
        requester: requester.clone(),
        model: model.clone(),
        mode: mode.clone(),
        input_hash: input_hash.clone(),
        max_fee: *max_fee,
        min_reputation: *min_reputation,
        bid_window_epochs: *bid_window_epochs,
        deadline_epoch: *deadline_epoch,
        posted_epoch: *epoch,
        status: JobStatus::Posted,
        winner: None,
        awarded_fee: None,
        verifiers: vec![],
        result_hash: None,
        latency_ms: None,
        disputed_epoch: None,
        claimed_epoch: None,
        settled_epoch: None,
        completed_epoch: None,
        persist_on_fs: persist_on_fs.unwrap_or(false),
        verdicts: vec![],
        commits: vec![],
        min_verifiers,
        node_fingerprint: node_fingerprint.clone(),
    })?;
    info!("inference job posted: {} model={} max_fee={} min_verifiers={}", job_id, model, max_fee, min_verifiers);
    Ok(())
}

pub fn apply_bid(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::InferenceJobBid { job_id, bidder, fee, role, epoch, .. } = entry
        else { bail!("wrong entry type") };

    let job = get_job(chain, job_id)
        .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;
    if job.status != JobStatus::Posted {
        bail!("job '{}' not open for bids (status: {:?})", job_id, job.status);
    }
    // Chain policy: bid window is closed once posted_epoch + bid_window_epochs has passed.
    // Use the chain's authoritative current epoch, not the epoch field the bidder supplied.
    let current_epoch = chain.current_epoch();
    if current_epoch >= job.posted_epoch + job.bid_window_epochs {
        bail!("bid window for job '{}' closed at epoch {}", job_id, job.posted_epoch + job.bid_window_epochs);
    }
    // Chain policy: requester cannot bid on their own job.
    if bidder == &job.requester {
        bail!("requester '{}' cannot bid on their own job '{}'", bidder, job_id);
    }
    if *fee > job.max_fee {
        bail!("bid fee {} exceeds job max_fee {}", fee, job.max_fee);
    }
    if !["worker", "verifier"].contains(&role.as_str()) {
        bail!("bid role must be 'worker' or 'verifier'");
    }
    let rep = get_reputation(chain, bidder);
    if rep.score < job.min_reputation {
        bail!("node '{}' reputation {} below required {}", bidder, rep.score, job.min_reputation);
    }
    if chain.store.state_get(&bid_key(job_id, bidder)).is_some() {
        bail!("node '{}' already bid on job '{}'", bidder, job_id);
    }
    set_bid(chain, &BidState {
        job_id: job_id.clone(),
        bidder: bidder.clone(),
        fee: *fee,
        role: role.clone(),
        epoch: *epoch,
    })?;
    Ok(())
}

pub fn apply_award(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::InferenceJobAward { job_id, winner, role, fee, .. } = entry
        else { bail!("wrong entry type") };

    let mut job = get_job(chain, job_id)
        .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;
    if job.status != JobStatus::Posted {
        bail!("job '{}' cannot be awarded (status: {:?})", job_id, job.status);
    }

    match role.as_str() {
        "worker" => {
            job.winner = Some(winner.clone());
            job.awarded_fee = Some(*fee);
            job.status = JobStatus::Awarded;
            // Index so the miner loop can find awarded jobs without scanning all jobs.
            let _ = chain.store.state_set(
                &format!("mine_queue:{}:{}", winner, job_id), b"awarded");
        }
        "verifier" => {
            // Chain policy: cap verifiers per job at 3 (matches daemon selection).
            if job.verifiers.len() >= 3 {
                bail!("job '{}' already has the maximum 3 verifiers", job_id);
            }
            if job.verifiers.contains(winner) {
                bail!("'{}' is already a verifier for job '{}'", winner, job_id);
            }
            job.verifiers.push(winner.clone());
        }
        _ => bail!("unknown award role '{}'", role),
    }

    let mut rep = get_reputation(chain, winner);
    rep.jobs_accepted += 1;
    rep.recompute_score();
    set_reputation(chain, &rep)?;
    set_job(chain, &job)?;
    info!("inference job awarded: {} → {} (role={})", job_id, winner, role);
    Ok(())
}

pub fn apply_complete(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::InferenceJobComplete { job_id, worker, result_hash, latency_ms, epoch, .. } = entry
        else { bail!("wrong entry type") };

    let mut job = get_job(chain, job_id)
        .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;
    if job.status != JobStatus::Awarded {
        bail!("job '{}' not in awarded state", job_id);
    }
    if job.winner.as_deref() != Some(worker.as_str()) {
        bail!("'{}' is not the awarded worker for job '{}'", worker, job_id);
    }
    // Staleness gate: reject completions submitted more than 3 epochs past deadline.
    // The deadline daemon cancels jobs at deadline+0, but a node with a stale view
    // might still try to complete a long-expired job.
    const COMPLETE_STALE_WINDOW: u64 = 3;
    let cur = chain.current_epoch();
    if cur > job.deadline_epoch + COMPLETE_STALE_WINDOW {
        bail!(
            "job '{}' completion is too late — deadline was epoch {}, current epoch {}",
            job_id, job.deadline_epoch, cur
        );
    }

    job.result_hash = Some(result_hash.clone());
    job.latency_ms = Some(*latency_ms);
    job.status = JobStatus::Completed;
    job.completed_epoch = Some(*epoch);

    let mut rep = get_reputation(chain, worker);
    rep.jobs_completed += 1;
    rep.total_latency_ms = rep.total_latency_ms.saturating_add(*latency_ms);
    rep.recompute_score();
    set_reputation(chain, &rep)?;
    set_job(chain, &job)?;
    // Mark queue entry so miner knows the job is ready for Mine submission.
    let _ = chain.store.state_set(
        &format!("mine_queue:{}:{}", worker, job_id), b"completed");
    Ok(())
}

/// Commit phase of commit-reveal (T4-2). Verifier commits sha256(verdict|salt)
/// before the reveal window opens. Prevents verdict copying among board members.
pub fn apply_commit(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::InferenceJobCommit { job_id, verifier, commit_hash, epoch, .. } = entry
        else { bail!("wrong entry type") };

    let mut job = get_job(chain, job_id)
        .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;

    if job.status != JobStatus::Completed {
        bail!("job '{}' must be Completed to accept commits (status: {:?})", job_id, job.status);
    }

    // Validate commit_hash is a valid hex sha256 (64 chars).
    if commit_hash.len() != 64 || !commit_hash.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("commit_hash must be a 64-char hex SHA-256");
    }

    // Reject duplicate commits from the same verifier.
    if job.commits.iter().any(|(v, _)| v == verifier) {
        bail!("verifier '{}' already committed for job '{}'", verifier, job_id);
    }

    // Check pre-assigned verifier list if present.
    if !job.verifiers.is_empty() && !job.verifiers.contains(verifier) {
        bail!("'{}' is not an assigned verifier for job '{}'", verifier, job_id);
    }

    job.commits.push((verifier.clone(), commit_hash.clone()));
    set_job(chain, &job)?;

    // Per-epoch stats for tracking which verifiers are active.
    let vkey = format!("infer_verify:{}:{}", epoch, verifier);
    let prev = chain.store.state_get(&vkey)
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let prev_count: u64 = prev.as_ref().and_then(|j| j["count"].as_u64()).unwrap_or(0);
    let prev_score: u64 = prev.as_ref().and_then(|j| j["value_score_total"].as_u64()).unwrap_or(0);
    let _ = chain.store.state_set(&vkey, &serde_json::to_vec(&serde_json::json!({
        "verifier": verifier, "epoch": epoch,
        "count": prev_count + 1,
        "value_score_total": prev_score,
    })).unwrap_or_default());

    info!("AI task {} commit from verifier {} ({}/{})", job_id, verifier, job.commits.len(), job.min_verifiers);
    Ok(())
}

pub fn apply_verify(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    use sha2::{Sha256, Digest};
    let LedgerEntry::InferenceJobVerify { job_id, verifier, verdict, value_score, epoch, reveal_salt, .. } = entry
        else { bail!("wrong entry type") };

    let mut job = get_job(chain, job_id)
        .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;
    if job.status != JobStatus::Completed {
        bail!("job '{}' not in completed state for verification", job_id);
    }
    // Chain policy: a node cannot verify its own work.
    if job.winner.as_deref() == Some(verifier.as_str()) {
        bail!("verifier '{}' cannot verify their own job '{}'", verifier, job_id);
    }

    // T4-2: Commit-reveal — verify must be preceded by a matching commit.
    if INFERENCE_COMMIT_REVEAL_ENABLED {
        let commit_hash = job.commits.iter()
            .find(|(v, _)| v == verifier)
            .map(|(_, h)| h.as_str());
        match commit_hash {
            None => bail!("verifier '{}' must submit InferenceJobCommit before revealing", verifier),
            Some(stored_hash) => {
                let salt = reveal_salt.as_deref().unwrap_or("");
                let expected = {
                    let mut h = Sha256::new();
                    h.update(verdict.as_bytes());
                    h.update(b"|");
                    h.update(salt.as_bytes());
                    hex::encode(h.finalize())
                };
                if expected != stored_hash {
                    bail!("reveal does not match commit for verifier '{}' on job '{}'", verifier, job_id);
                }
            }
        }
    }

    // Suspension check.
    let rep_key = format!("verifier_rep:{}", verifier);
    let vrep: serde_json::Value = chain.store.state_get(&rep_key)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if vrep["suspended_until"].as_u64().unwrap_or(0) > *epoch {
        bail!("verifier '{}' is suspended until epoch {}", verifier, vrep["suspended_until"]);
    }

    // D9 random assignment check.
    if VERIFIER_ASSIGNMENT_ENABLED {
        let entropy = chain.store.state_get(&format!("epoch_entropy:{}", epoch))
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();
        let mut h = Sha256::new();
        h.update(format!("{}:{}:{}", entropy, job_id, verifier).as_bytes());
        let hash = h.finalize();
        if hash[0] >= VERIFIER_THRESHOLD_BYTE {
            bail!("verifier '{}' not assigned to job '{}' this epoch", verifier, job_id);
        }
    }

    // Pre-assigned verifier check.
    if !job.verifiers.is_empty() && !job.verifiers.contains(verifier) {
        bail!("'{}' is not an assigned verifier for job '{}'", verifier, job_id);
    }

    // Board: reject duplicate votes.
    if job.verdicts.iter().any(|(v, _)| v == verifier) {
        bail!("verifier '{}' already submitted a verdict for '{}'", verifier, job_id);
    }

    // Validate verdict string.
    if !["approved", "rejected", "review_required"].contains(&verdict.as_str()) {
        bail!("unknown verdict '{}' (expected approved|rejected|review_required)", verdict);
    }

    // Accumulate verdict into board.
    job.verdicts.push((verifier.clone(), verdict.clone()));

    // Check for board quorum — resolve only when enough verifiers have voted.
    if let Some(consensus) = resolve_quorum_verdict(&job.verdicts, job.min_verifiers) {
        match consensus {
            "approved" => {
                job.status = JobStatus::Verified;
                info!("AI task {} board verdict: approved ({}/{} votes)", job_id, job.verdicts.len(), job.min_verifiers);
            }
            "rejected" => {
                job.status = JobStatus::Rejected;
                if let Some(ref winner) = job.winner.clone() {
                    let mut rep = get_reputation(chain, winner);
                    rep.jobs_failed += 1;
                    rep.recompute_score();
                    set_reputation(chain, &rep)?;
                }
                info!("AI task {} board verdict: rejected ({}/{} votes)", job_id, job.verdicts.len(), job.min_verifiers);
            }
            "review_required" => {
                job.status = JobStatus::Disputed;
                job.disputed_epoch = Some(*epoch);
                info!("AI task {} board verdict: disputed ({}/{} votes)", job_id, job.verdicts.len(), job.min_verifiers);
            }
            _ => {}
        }
    }
    // If no quorum yet: status stays Completed, waiting for more verifiers.
    set_job(chain, &job)?;

    // Per-epoch stats for verifier reward calculation.
    let vkey = format!("infer_verify:{}:{}", epoch, verifier);
    let prev = chain.store.state_get(&vkey)
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let prev_count: u64 = prev.as_ref().and_then(|j| j["count"].as_u64()).unwrap_or(0);
    let prev_score: u64 = prev.as_ref().and_then(|j| j["value_score_total"].as_u64()).unwrap_or(0);
    let _ = chain.store.state_set(&vkey,
        &serde_json::to_vec(&serde_json::json!({
            "verifier": verifier, "epoch": epoch,
            "count": prev_count + 1,
            "value_score_total": prev_score + value_score,
        })).unwrap_or_default());

    // Leaky-bucket approval-rate tracking (rubber-stamp detection).
    update_verifier_approval_rate(chain, verifier, verdict == "approved")?;

    Ok(())
}

/// Update the leaky-bucket approval-rate window for a verifier and apply weight/suspension.
///
/// Leaky bucket (same model as clock uptime): `total` grows until WINDOW, then holds steady.
/// In steady state, a rejection decrements `approved_count`; an approval leaves it unchanged
/// (evict one old approval, add one new approval = net zero). This approximates a rolling
/// window without storing full history and cannot be gamed by pausing submissions.
pub fn update_verifier_approval_rate_pub(chain: &Chain, verifier: &str, approved: bool) -> anyhow::Result<()> {
    update_verifier_approval_rate(chain, verifier, approved)
}
fn update_verifier_approval_rate(chain: &Chain, verifier: &str, approved: bool) -> anyhow::Result<()> {
    let rep_key = format!("verifier_rep:{}", verifier);
    let mut vrep: serde_json::Value = chain.store.state_get(&rep_key)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({
            "total": 0u64,
            "approved_count": 0u64,
            "weight_halved": false,
            "suspended_until": 0u64,
        }));

    let prev_total = vrep["total"].as_u64().unwrap_or(0);
    let prev_approved = vrep["approved_count"].as_u64().unwrap_or(0);

    let (total, approved_count) = if prev_total < VERIFIER_APPROVAL_WINDOW_EPOCHS {
        // Growing phase: accumulate.
        (prev_total + 1, prev_approved + if approved { 1 } else { 0 })
    } else {
        // Steady state: evict one old verdict, add new one.
        // Approved verdict: evict assumed-approved → net zero change on approved_count.
        // Rejected verdict: evict assumed-approved → approved_count - 1.
        let new_approved = if approved { prev_approved } else { prev_approved.saturating_sub(1) };
        (prev_total, new_approved)
    };

    let approval_bps = if total > 0 { approved_count * 10_000 / total } else { 0 };
    let prev_suspended_until = vrep["suspended_until"].as_u64().unwrap_or(0);

    // Only fire checks after 10 verdicts to avoid early false positives.
    let (weight_halved, suspended_until) = if total >= 10 {
        if approval_bps >= VERIFIER_SUSPEND_BPS {
            // Derive a synthetic epoch for the log message from the suspended_until delta.
            tracing::warn!(
                "[verifier] '{}' suspended (approval rate {}% over {} verdicts)",
                verifier, approval_bps / 100, total
            );
            (true, prev_suspended_until.max(total + VERIFIER_SUSPEND_EPOCHS))
        } else if approval_bps >= VERIFIER_RUBBER_STAMP_BPS {
            (true, prev_suspended_until)
        } else {
            (false, prev_suspended_until)
        }
    } else {
        (false, prev_suspended_until)
    };

    vrep["total"] = serde_json::json!(total);
    vrep["approved_count"] = serde_json::json!(approved_count);
    vrep["weight_halved"] = serde_json::json!(weight_halved);
    vrep["suspended_until"] = serde_json::json!(suspended_until);
    chain.store.state_set(&rep_key, &serde_json::to_vec(&vrep)?)?;
    Ok(())
}

pub fn apply_claim(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::InferenceJobClaim { job_id, claimant, epoch, .. } = entry
        else { bail!("wrong entry type") };

    let mut job = get_job(chain, job_id)
        .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;
    if job.status != JobStatus::Disputed {
        bail!("job '{}' is not in review_required state", job_id);
    }
    let is_worker    = job.winner.as_deref() == Some(claimant.as_str());
    let is_requester = job.requester == *claimant;
    if !is_worker && !is_requester {
        bail!("only the worker or requester can file a claim on job '{}'", job_id);
    }
    let disputed_at = job.disputed_epoch.unwrap_or(0);
    if *epoch > disputed_at + CLAIM_WINDOW_EPOCHS {
        bail!("claim window for job '{}' has expired", job_id);
    }

    job.status = JobStatus::Claimed;
    job.claimed_epoch = Some(*epoch);
    set_job(chain, &job)?;
    // nonce bump in tx.rs
    info!("inference job claimed: {} by {}", job_id, claimant);
    Ok(())
}

pub fn apply_review_vote(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::InferenceReviewVote { job_id, reviewer, approved, epoch, .. } = entry
        else { bail!("wrong entry type") };

    let mut job = get_job(chain, job_id)
        .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;
    if job.status != JobStatus::Claimed {
        bail!("job '{}' is not in claimed state", job_id);
    }

    let mut votes = get_votes(chain, job_id);
    if votes.iter().any(|v| v.reviewer == *reviewer) {
        bail!("reviewer '{}' already voted on job '{}'", reviewer, job_id);
    }
    votes.push(ReviewVote { reviewer: reviewer.clone(), approved: *approved, epoch: *epoch });
    set_votes(chain, job_id, &votes)?;

    // Resolve once minimum votes reached.
    if votes.len() as u64 >= MIN_REVIEW_VOTES {
        let approvals = votes.iter().filter(|v| v.approved).count();
        let rejections = votes.len() - approvals;
        if approvals > rejections {
            job.status = JobStatus::Reviewed;
        } else {
            job.status = JobStatus::Rejected;
        }
        set_job(chain, &job)?;
    }
    Ok(())
}

pub fn apply_pay(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::InferenceJobPay {
        job_id, worker, worker_amount, verifier_payments, reviewer_payments,
        recycle_amount, refund_amount, dissenter_slashes, epoch, ..
    } = entry else { bail!("wrong entry type") };

    let mut job = get_job(chain, job_id)
        .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;

    let allowed = matches!(job.status, JobStatus::Verified | JobStatus::Reviewed | JobStatus::NoFee | JobStatus::Rejected);
    if !allowed {
        bail!("job '{}' not ready for payment (status: {:?})", job_id, job.status);
    }

    // All funds sit in RECYCLE_FUND_ACCOUNT as escrow.
    let total_out = worker_amount
        + verifier_payments.iter().map(|(_, a)| a).sum::<u64>()
        + reviewer_payments.iter().map(|(_, a)| a).sum::<u64>()
        + recycle_amount
        + refund_amount;
    chain.store.debit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, total_out)?;

    if *worker_amount > 0 {
        chain.store.credit(worker, NATIVE_TOKEN, *worker_amount)?;
    }
    for (account, amount) in verifier_payments {
        chain.store.credit(account, NATIVE_TOKEN, *amount)?;
    }
    for (account, amount) in reviewer_payments {
        chain.store.credit(account, NATIVE_TOKEN, *amount)?;
    }
    // recycle_amount stays in the fund (debit+credit is a no-op net; skip for efficiency).
    chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, *recycle_amount)?;
    if *refund_amount > 0 {
        chain.store.credit(&job.requester, NATIVE_TOKEN, *refund_amount)?;
    }

    // Slash dissenters: debit from staked balance, credit to recycle fund.
    // Best-effort — capped at their actual stake; never fails the whole pay entry.
    for (dissenter, slash_amount) in dissenter_slashes {
        let current_stake = chain.store.get_stake(dissenter);
        let actual_slash = (*slash_amount).min(current_stake);
        if actual_slash > 0 {
            let _ = chain.store.set_stake(dissenter, current_stake - actual_slash);
            let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, actual_slash);
            info!("verifier dissent slash: {} -{} stake → recycle", dissenter, actual_slash);
        }
    }

    job.settled_epoch = Some(*epoch);
    job.status = JobStatus::Paid;
    set_job(chain, &job)?;

    // Schedule proof payload pruning (D1): mark for clearing after prune window.
    // Verdict and reward records stay forever; only raw proof data is cleared.
    if !job.persist_on_fs {
        let prune_at = epoch + INFERENCE_PRUNE_WINDOW_EPOCHS;
        let prune_key = format!("prune_ready:{}:{}", prune_at, job_id);
        let _ = chain.store.state_set(&prune_key, job_id.as_bytes());
    }

    info!("inference job paid: {} worker={} +{}", job_id, worker, worker_amount);
    Ok(())
}

pub fn apply_cancel(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::InferenceJobCancel { job_id, cancelled_by, .. } = entry
        else { bail!("wrong entry type") };

    let mut job = get_job(chain, job_id)
        .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;
    if !matches!(job.status, JobStatus::Posted | JobStatus::Awarded) {
        bail!("job '{}' cannot be cancelled (status: {:?})", job_id, job.status);
    }
    match cancelled_by.as_str() {
        "system" => {} // system always allowed (deadline expiry)
        cb if cb == job.requester => {
            // Chain policy: requester can only cancel before a worker is assigned.
            // Once Awarded, only the system can cancel (after deadline).
            if job.status == JobStatus::Awarded {
                bail!("requester cannot cancel job '{}' after it has been awarded — wait for deadline", job_id);
            }
        }
        _ => bail!("only the requester or system can cancel job '{}'", job_id),
    }

    chain.store.debit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, job.max_fee)?;
    chain.store.credit(&job.requester, NATIVE_TOKEN, job.max_fee)?;

    if let Some(ref winner) = job.winner.clone() {
        let mut rep = get_reputation(chain, winner);
        rep.jobs_failed += 1;
        rep.recompute_score();
        set_reputation(chain, &rep)?;
    }

    job.status = JobStatus::Cancelled;
    set_job(chain, &job)?;
    Ok(())
}

// ── Award/pay builder helpers ─────────────────────────────────────────────────

pub fn select_best_bid(bids: &[BidState], chain: &Chain, role: &str) -> Option<BidState> {
    bids.iter()
        .filter(|b| b.role == role)
        .max_by_key(|b| {
            let rep = get_reputation(chain, &b.bidder).score;
            rep.saturating_mul(10_000) / b.fee.max(1)
        })
        .cloned()
}

/// Split board verdicts into consensus verifiers and dissenters for a given consensus verdict.
/// Returns (consensus_accounts, dissenter_accounts).
fn split_board_by_consensus<'a>(job: &'a JobState, consensus: &str) -> (Vec<&'a str>, Vec<&'a str>) {
    if job.verdicts.is_empty() {
        // Legacy job or no board — treat all verifiers as consensus, no dissenters.
        return (job.verifiers.iter().map(String::as_str).collect(), vec![]);
    }
    let consensus_vers = job.verdicts.iter()
        .filter(|(_, v)| v.as_str() == consensus)
        .map(|(acc, _)| acc.as_str())
        .collect();
    let dissenters = job.verdicts.iter()
        .filter(|(_, v)| v.as_str() != consensus)
        .map(|(acc, _)| acc.as_str())
        .collect();
    (consensus_vers, dissenters)
}

/// Read weight_halved flag for a verifier from their rep record.
fn verifier_weight(chain: &Chain, verifier: &str) -> u64 {
    let vrep: serde_json::Value = chain.store.state_get(&format!("verifier_rep:{}", verifier))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if vrep["weight_halved"].as_bool().unwrap_or(false) { 50 } else { 100 }
}

/// Compute weighted verifier payments and dissenter slashes for a pool.
/// Consensus verifiers split the pool by weight; dissenters get 0 and are slashed
/// their equal would-have-earned share from staked balance.
fn compute_verifier_split(
    chain: &Chain,
    consensus_vers: &[&str],
    dissenters: &[&str],
    pool: u64,
) -> (Vec<(String, u64)>, Vec<(String, u64)>) {
    let total_ver_count = (consensus_vers.len() + dissenters.len()).max(1) as u64;
    let per_head = pool / total_ver_count; // equal share each verifier would have earned

    // Weighted split among consensus verifiers.
    let weights: Vec<u64> = consensus_vers.iter().map(|v| verifier_weight(chain, v)).collect();
    let total_weight = weights.iter().sum::<u64>().max(1);
    let payments = consensus_vers.iter().zip(weights.iter())
        .map(|(acc, w)| (acc.to_string(), pool * w / total_weight / 1)) // proportional to weight
        .collect::<Vec<_>>();

    // Dissenters: 0 pay; slashed for their equal share.
    let slashes = dissenters.iter()
        .map(|acc| (acc.to_string(), per_head))
        .collect::<Vec<_>>();

    (payments, slashes)
}

/// Build InferenceJobPay for the happy path (Verified, board consensus = approved).
pub fn build_pay_entry_happy(chain: &Chain, job: &JobState, epoch: u64) -> Option<LedgerEntry> {
    let worker = job.winner.as_ref()?;
    let fee = job.awarded_fee?;

    let worker_amount = fee * INFERENCE_FEE_WORKER_BPS / 10_000;
    let verifier_pool = fee * INFERENCE_FEE_VERIFIER_BPS / 10_000;
    let recycle_amount = fee * INFERENCE_FEE_RECYCLE_BPS / 10_000;

    let (consensus_vers, dissenters) = split_board_by_consensus(job, "approved");
    let (verifier_payments, dissenter_slashes) =
        compute_verifier_split(chain, &consensus_vers, &dissenters, verifier_pool);

    let paid: u64 = worker_amount
        + verifier_payments.iter().map(|(_, a)| *a).sum::<u64>()
        + recycle_amount;
    let refund_amount = job.max_fee.saturating_sub(paid);

    Some(LedgerEntry::InferenceJobPay {
        job_id: job.job_id.clone(),
        worker: worker.clone(),
        worker_amount,
        verifier_payments,
        reviewer_payments: vec![],
        recycle_amount,
        refund_amount,
        dissenter_slashes,
        epoch,
    })
}

/// Build InferenceJobPay for the disputed+reviewed path (worker wins review).
pub fn build_pay_entry_disputed(
    chain: &Chain,
    job: &JobState,
    reviewers: &[String],
    epoch: u64,
) -> Option<LedgerEntry> {
    let worker = job.winner.as_ref()?;
    let fee = job.awarded_fee?;

    let worker_amount = fee * INFERENCE_FEE_WORKER_DISPUTED_BPS / 10_000;
    let verifier_pool = fee * INFERENCE_FEE_VERIFIER_DISPUTED_BPS / 10_000;
    let reviewer_pool = fee * INFERENCE_FEE_REVIEWER_BPS / 10_000;
    let recycle_amount = fee * INFERENCE_FEE_RECYCLE_DISPUTED_BPS / 10_000;

    // On the disputed path, board verdicts caused the dispute.
    // Pay verifiers who voted review_required; slash those who voted approved/rejected.
    let (consensus_vers, dissenters) = split_board_by_consensus(job, "review_required");
    let (verifier_payments, dissenter_slashes) =
        compute_verifier_split(chain, &consensus_vers, &dissenters, verifier_pool);

    let n_rev = reviewers.len().max(1) as u64;
    let reviewer_payments = reviewers.iter()
        .map(|r| (r.clone(), reviewer_pool / n_rev))
        .collect::<Vec<_>>();

    let paid: u64 = worker_amount
        + verifier_payments.iter().map(|(_, a)| *a).sum::<u64>()
        + reviewer_payments.iter().map(|(_, a)| *a).sum::<u64>()
        + recycle_amount;
    let refund_amount = job.max_fee.saturating_sub(paid);

    Some(LedgerEntry::InferenceJobPay {
        job_id: job.job_id.clone(),
        worker: worker.clone(),
        worker_amount,
        verifier_payments,
        reviewer_payments,
        recycle_amount,
        refund_amount,
        dissenter_slashes,
        epoch,
    })
}

/// Return the quorum verdict for a board, exposed for tests.
#[cfg(test)]
pub fn quorum_verdict_for_test(verdicts: &[(String, String)], min: u64) -> Option<&'static str> {
    resolve_quorum_verdict(verdicts, min)
}

/// Build InferenceJobPay for the NoFee / Rejected path (worker gets nothing).
/// Verifiers and reviewers still get paid; remainder refunded to requester.
pub fn build_pay_entry_nofee(
    chain: &Chain,
    job: &JobState,
    reviewers: &[String],
    epoch: u64,
) -> LedgerEntry {
    let fee = job.awarded_fee.unwrap_or(job.max_fee);
    let verifier_pool = fee * INFERENCE_FEE_VERIFIER_DISPUTED_BPS / 10_000;
    let reviewer_pool = if reviewers.is_empty() { 0 } else { fee * INFERENCE_FEE_REVIEWER_BPS / 10_000 };
    let recycle_amount = fee * INFERENCE_FEE_RECYCLE_DISPUTED_BPS / 10_000;

    // On rejected path, verifiers who voted rejected are consensus; others dissent.
    let (consensus_vers, dissenters) = split_board_by_consensus(job, "rejected");
    let (verifier_payments, dissenter_slashes) =
        compute_verifier_split(chain, &consensus_vers, &dissenters, verifier_pool);

    let n_rev = reviewers.len().max(1) as u64;
    let reviewer_payments = reviewers.iter()
        .map(|r| (r.clone(), reviewer_pool / n_rev))
        .collect::<Vec<_>>();

    let paid: u64 = verifier_payments.iter().map(|(_, a)| *a).sum::<u64>()
        + reviewer_payments.iter().map(|(_, a)| *a).sum::<u64>()
        + recycle_amount;
    let refund_amount = job.max_fee.saturating_sub(paid);

    let worker = job.winner.clone().unwrap_or_else(|| "__no_worker__".to_string());

    LedgerEntry::InferenceJobPay {
        job_id: job.job_id.clone(),
        worker,
        worker_amount: 0,
        verifier_payments,
        reviewer_payments,
        recycle_amount,
        refund_amount,
        dissenter_slashes,
        epoch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use btcpc_types::{LedgerEntry, RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN,
        INFERENCE_FEE_WORKER_BPS, INFERENCE_FEE_VERIFIER_BPS, INFERENCE_FEE_RECYCLE_BPS};

    // ── Test helpers ─────────────────────────────────────────────────────────

    fn make_chain(label: &str) -> (Arc<Chain>, tempfile::TempDir) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("btcpc_infer_{}_", label))
            .tempdir()
            .expect("tempdir");
        let store = crate::store::Store::open(dir.path()).expect("store");
        let chain = Arc::new(Chain::new(
            store,
            format!("infer-{}", label),
            "btcpc-satoshi".to_string(),
        ));
        (chain, dir)
    }

    /// Seal epoch 0 so chain.current_epoch() == 0.
    fn seal(chain: &Chain, epoch: u64) {
        chain.apply_entry(&LedgerEntry::EpochSeal {
            node_id: "test".into(),
            epoch,
            timestamp: epoch * 30_000,
            seal_hash: format!("{:064x}", epoch),
            signature: None,
            node_version: None,
        }).unwrap();
    }

    /// Fund an account directly through the store.
    fn fund(chain: &Chain, account: &str, amount: u64) {
        chain.store.credit(account, NATIVE_TOKEN, amount).unwrap();
    }

    fn post_entry(job_id: &str, requester: &str, max_fee: u64, epoch: u64) -> LedgerEntry {
        LedgerEntry::InferenceJobPost {
            job_id: job_id.to_string(),
            requester: requester.to_string(),
            model: "llama3".to_string(),
            mode: "solo".to_string(),
            input_hash: "abc123".to_string(),
            max_fee,
            min_reputation: 0,
            bid_window_epochs: 2,
            deadline_epoch: epoch + 10,
            epoch,
            nonce: 1,
            signed_by: requester.to_string(),
            persist_on_fs: None,
            fs_fee: None,
            min_verifiers: None,
            node_fingerprint: None,
        }
    }

    fn bid_entry(job_id: &str, bidder: &str, fee: u64, role: &str, epoch: u64) -> LedgerEntry {
        LedgerEntry::InferenceJobBid {
            job_id: job_id.to_string(),
            bidder: bidder.to_string(),
            fee,
            role: role.to_string(),
            epoch,
            nonce: 1,
            signed_by: bidder.to_string(),
        }
    }

    fn award_entry(job_id: &str, winner: &str, fee: u64, role: &str) -> LedgerEntry {
        LedgerEntry::InferenceJobAward {
            job_id: job_id.to_string(),
            winner: winner.to_string(),
            role: role.to_string(),
            fee,
            epoch: 0,
        }
    }

    fn complete_entry(job_id: &str, worker: &str, epoch: u64) -> LedgerEntry {
        LedgerEntry::InferenceJobComplete {
            job_id: job_id.to_string(),
            worker: worker.to_string(),
            result_hash: "deadbeef".to_string(),
            latency_ms: 250,
            epoch,
            signed_by: worker.to_string(),
        }
    }

    fn verify_entry(job_id: &str, verifier: &str, verdict: &str, epoch: u64) -> LedgerEntry {
        LedgerEntry::InferenceJobVerify {
            job_id: job_id.to_string(),
            verifier: verifier.to_string(),
            verdict: verdict.to_string(),
            value_score: 100,
            reason: None,
            reveal_salt: None,
            epoch,
            signed_by: verifier.to_string(),
        }
    }

    // ── NodeReputation ────────────────────────────────────────────────────────

    #[test]
    fn reputation_new_starts_at_5000() {
        let rep = NodeReputation::new("alice");
        assert_eq!(rep.score, 5000);
    }

    #[test]
    fn reputation_perfect_completion_high_score() {
        let mut rep = NodeReputation::new("alice");
        rep.jobs_accepted = 10;
        rep.jobs_completed = 10;
        rep.total_latency_ms = 500; // avg 50ms → latency_factor = min(5000/50, 100) = 100
        rep.recompute_score();
        // completion=100, latency_factor=100 → score = min(100*(50+100), 10000) = 10000
        assert_eq!(rep.score, 10000);
    }

    #[test]
    fn reputation_zero_accepted_resets_to_5000() {
        let mut rep = NodeReputation::new("alice");
        rep.jobs_accepted = 0;
        rep.jobs_completed = 5;
        rep.recompute_score();
        assert_eq!(rep.score, 5000);
    }

    #[test]
    fn reputation_poor_completion_low_score() {
        let mut rep = NodeReputation::new("node");
        rep.jobs_accepted = 10;
        rep.jobs_completed = 2;
        rep.total_latency_ms = 50_000; // avg 25000ms → latency_factor = 5000/25000 = 0 (min 1)
        rep.recompute_score();
        // completion = 20, latency_factor ≈ 0 → score = 20 * 50 = 1000
        assert!(rep.score < 2000, "poor completion should give low score");
    }

    // ── Quorum verdict ────────────────────────────────────────────────────────

    #[test]
    fn quorum_approved_with_one_vote_min_one() {
        let verdicts = vec![("v1".into(), "approved".into())];
        assert_eq!(quorum_verdict_for_test(&verdicts, 1), Some("approved"));
    }

    #[test]
    fn quorum_not_reached_with_one_of_three_needed() {
        let verdicts = vec![("v1".into(), "approved".into())];
        // 1 vote, 2 remaining; approved can still be beaten → no decision
        assert_eq!(quorum_verdict_for_test(&verdicts, 3), None);
    }

    #[test]
    fn quorum_review_required_wins_ties() {
        // Three votes: one each — review_required has +1 weight so it should win.
        let verdicts = vec![
            ("v1".into(), "approved".into()),
            ("v2".into(), "rejected".into()),
            ("v3".into(), "review_required".into()),
        ];
        assert_eq!(quorum_verdict_for_test(&verdicts, 3), Some("review_required"));
    }

    #[test]
    fn quorum_majority_approved() {
        let verdicts = vec![
            ("v1".into(), "approved".into()),
            ("v2".into(), "approved".into()),
            ("v3".into(), "rejected".into()),
        ];
        assert_eq!(quorum_verdict_for_test(&verdicts, 3), Some("approved"));
    }

    // ── apply_post ────────────────────────────────────────────────────────────

    #[test]
    fn post_creates_job_and_debits_fee() {
        let (chain, _dir) = make_chain("post");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);

        let entry = post_entry("j1", "alice", 1_000, 0);
        apply_post(&chain, &entry).expect("post");

        let job = get_job(&chain, "j1").expect("job created");
        assert_eq!(job.status, JobStatus::Posted);
        assert_eq!(job.max_fee, 1_000);
        assert_eq!(job.requester, "alice");

        // Fee debited from alice, held in recycle fund as escrow.
        assert_eq!(chain.store.get_balance("alice", NATIVE_TOKEN), 9_000);
        assert_eq!(chain.store.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN), 1_000);
    }

    #[test]
    fn post_duplicate_job_id_fails() {
        let (chain, _dir) = make_chain("post_dup");
        seal(&chain, 0);
        fund(&chain, "alice", 20_000);

        let entry = post_entry("j-dup", "alice", 1_000, 0);
        apply_post(&chain, &entry).expect("first post");
        assert!(apply_post(&chain, &entry).is_err(), "duplicate job_id must fail");
    }

    #[test]
    fn post_insufficient_balance_fails() {
        let (chain, _dir) = make_chain("post_insufficient");
        seal(&chain, 0);
        // alice has 0 balance

        let entry = post_entry("j-broke", "alice", 500, 0);
        assert!(apply_post(&chain, &entry).is_err());
    }

    // ── apply_bid ─────────────────────────────────────────────────────────────

    #[test]
    fn bid_accepted_within_window() {
        let (chain, _dir) = make_chain("bid_ok");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();

        apply_bid(&chain, &bid_entry("j1", "bob", 800, "worker", 0)).expect("bid ok");
        let bids = get_bids(&chain, "j1");
        assert_eq!(bids.len(), 1);
        assert_eq!(bids[0].bidder, "bob");
    }

    #[test]
    fn bid_requester_cannot_self_bid() {
        let (chain, _dir) = make_chain("bid_self");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();

        assert!(apply_bid(&chain, &bid_entry("j1", "alice", 800, "worker", 0)).is_err());
    }

    #[test]
    fn bid_fee_above_max_rejected() {
        let (chain, _dir) = make_chain("bid_highfee");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();

        assert!(apply_bid(&chain, &bid_entry("j1", "bob", 1_500, "worker", 0)).is_err());
    }

    #[test]
    fn bid_duplicate_rejected() {
        let (chain, _dir) = make_chain("bid_dup");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();

        apply_bid(&chain, &bid_entry("j1", "bob", 800, "worker", 0)).unwrap();
        assert!(apply_bid(&chain, &bid_entry("j1", "bob", 800, "worker", 0)).is_err());
    }

    #[test]
    fn bid_invalid_role_rejected() {
        let (chain, _dir) = make_chain("bid_role");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();

        assert!(apply_bid(&chain, &bid_entry("j1", "bob", 800, "reviewer", 0)).is_err());
    }

    #[test]
    fn bid_reputation_gate() {
        let (chain, _dir) = make_chain("bid_rep");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);

        // Post with min_reputation = 7000
        let post = LedgerEntry::InferenceJobPost {
            job_id: "j-rep".into(),
            requester: "alice".into(),
            model: "llama3".into(),
            mode: "solo".into(),
            input_hash: "x".into(),
            max_fee: 1_000,
            min_reputation: 7_000,
            bid_window_epochs: 2,
            deadline_epoch: 10,
            epoch: 0,
            nonce: 1,
            signed_by: "alice".into(),
            persist_on_fs: None,
            fs_fee: None,
            min_verifiers: None,
            node_fingerprint: None,
        };
        apply_post(&chain, &post).unwrap();

        // Bob starts with 5000 reputation (default) — below 7000 → bid rejected.
        assert!(apply_bid(&chain, &bid_entry("j-rep", "bob", 800, "worker", 0)).is_err());
    }

    // ── apply_award ───────────────────────────────────────────────────────────

    #[test]
    fn award_worker_transitions_to_awarded() {
        let (chain, _dir) = make_chain("award");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_bid(&chain, &bid_entry("j1", "bob", 800, "worker", 0)).unwrap();

        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).expect("award");
        let job = get_job(&chain, "j1").unwrap();
        assert_eq!(job.status, JobStatus::Awarded);
        assert_eq!(job.winner.as_deref(), Some("bob"));
        assert_eq!(job.awarded_fee, Some(800));
    }

    #[test]
    fn award_verifier_adds_to_list() {
        let (chain, _dir) = make_chain("award_ver");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_bid(&chain, &bid_entry("j1", "carol", 500, "verifier", 0)).unwrap();

        apply_award(&chain, &award_entry("j1", "carol", 500, "verifier")).expect("verifier award");
        let job = get_job(&chain, "j1").unwrap();
        assert_eq!(job.status, JobStatus::Posted, "verifier award does not change status");
        assert!(job.verifiers.contains(&"carol".to_string()));
    }

    #[test]
    fn award_double_worker_fails() {
        let (chain, _dir) = make_chain("award_double");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();

        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();
        // Second award on an already-Awarded job must fail.
        assert!(apply_award(&chain, &award_entry("j1", "carol", 700, "worker")).is_err());
    }

    #[test]
    fn award_bumps_reputation() {
        let (chain, _dir) = make_chain("award_rep");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_bid(&chain, &bid_entry("j1", "bob", 800, "worker", 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();

        let rep = get_reputation(&chain, "bob");
        assert_eq!(rep.jobs_accepted, 1);
    }

    // ── apply_complete ────────────────────────────────────────────────────────

    #[test]
    fn complete_transitions_to_completed() {
        let (chain, _dir) = make_chain("complete");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();

        apply_complete(&chain, &complete_entry("j1", "bob", 0)).expect("complete");
        let job = get_job(&chain, "j1").unwrap();
        assert_eq!(job.status, JobStatus::Completed);
        assert_eq!(job.result_hash.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn complete_wrong_worker_fails() {
        let (chain, _dir) = make_chain("complete_wrong");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();

        assert!(apply_complete(&chain, &complete_entry("j1", "carol", 0)).is_err());
    }

    #[test]
    fn complete_updates_worker_reputation() {
        let (chain, _dir) = make_chain("complete_rep");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j1", "bob", 0)).unwrap();

        let rep = get_reputation(&chain, "bob");
        assert_eq!(rep.jobs_completed, 1);
        assert!(rep.total_latency_ms > 0);
    }

    // ── apply_verify (happy path) ─────────────────────────────────────────────

    #[test]
    fn verify_approved_transitions_to_verified() {
        let (chain, _dir) = make_chain("verify_ok");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j1", "bob", 0)).unwrap();

        // min_verifiers = 1 (auto-scaled from empty network)
        apply_verify(&chain, &verify_entry("j1", "verifier1", "approved", 0)).expect("verify");
        let job = get_job(&chain, "j1").unwrap();
        assert_eq!(job.status, JobStatus::Verified);
    }

    #[test]
    fn verify_rejected_transitions_to_rejected() {
        let (chain, _dir) = make_chain("verify_reject");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j1", "bob", 0)).unwrap();

        apply_verify(&chain, &verify_entry("j1", "verifier1", "rejected", 0)).unwrap();
        let job = get_job(&chain, "j1").unwrap();
        assert_eq!(job.status, JobStatus::Rejected);

        // Rejection increments worker's jobs_failed.
        let rep = get_reputation(&chain, "bob");
        assert_eq!(rep.jobs_failed, 1);
    }

    #[test]
    fn verify_review_required_transitions_to_disputed() {
        let (chain, _dir) = make_chain("verify_dispute");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j1", "bob", 0)).unwrap();

        apply_verify(&chain, &verify_entry("j1", "verifier1", "review_required", 0)).unwrap();
        let job = get_job(&chain, "j1").unwrap();
        assert_eq!(job.status, JobStatus::Disputed);
        assert!(job.disputed_epoch.is_some());
    }

    #[test]
    fn verify_worker_cannot_self_verify() {
        let (chain, _dir) = make_chain("verify_self");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j1", "bob", 0)).unwrap();

        assert!(apply_verify(&chain, &verify_entry("j1", "bob", "approved", 0)).is_err());
    }

    #[test]
    fn verify_duplicate_rejected() {
        let (chain, _dir) = make_chain("verify_dup");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);

        // Post with min_verifiers=2 so first vote doesn't resolve.
        let post = LedgerEntry::InferenceJobPost {
            job_id: "j-v2".into(),
            requester: "alice".into(),
            model: "llama3".into(),
            mode: "solo".into(),
            input_hash: "x".into(),
            max_fee: 1_000,
            min_reputation: 0,
            bid_window_epochs: 2,
            deadline_epoch: 10,
            epoch: 0,
            nonce: 1,
            signed_by: "alice".into(),
            persist_on_fs: None,
            fs_fee: None,
            min_verifiers: Some(2),
            node_fingerprint: None,
        };
        apply_post(&chain, &post).unwrap();
        apply_award(&chain, &award_entry("j-v2", "bob", 800, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j-v2", "bob", 0)).unwrap();

        apply_verify(&chain, &verify_entry("j-v2", "v1", "approved", 0)).unwrap();
        assert!(apply_verify(&chain, &verify_entry("j-v2", "v1", "approved", 0)).is_err());
    }

    // ── apply_claim ───────────────────────────────────────────────────────────

    #[test]
    fn claim_by_worker_accepted() {
        let (chain, _dir) = make_chain("claim");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j1", "bob", 0)).unwrap();
        apply_verify(&chain, &verify_entry("j1", "v1", "review_required", 0)).unwrap();

        let claim = LedgerEntry::InferenceJobClaim {
            job_id: "j1".into(),
            claimant: "bob".into(),
            evidence_hash: None,
            epoch: 1,
            nonce: 1,
            signed_by: "bob".into(),
        };
        apply_claim(&chain, &claim).expect("claim ok");
        let job = get_job(&chain, "j1").unwrap();
        assert_eq!(job.status, JobStatus::Claimed);
    }

    #[test]
    fn claim_expired_window_fails() {
        let (chain, _dir) = make_chain("claim_expired");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j1", "bob", 0)).unwrap();
        apply_verify(&chain, &verify_entry("j1", "v1", "review_required", 0)).unwrap();

        // Dispute at epoch 0; CLAIM_WINDOW_EPOCHS = 20; claim at epoch 21 → expired.
        let claim = LedgerEntry::InferenceJobClaim {
            job_id: "j1".into(),
            claimant: "bob".into(),
            evidence_hash: None,
            epoch: 21,
            nonce: 1,
            signed_by: "bob".into(),
        };
        assert!(apply_claim(&chain, &claim).is_err());
    }

    #[test]
    fn claim_third_party_fails() {
        let (chain, _dir) = make_chain("claim_stranger");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j1", "bob", 0)).unwrap();
        apply_verify(&chain, &verify_entry("j1", "v1", "review_required", 0)).unwrap();

        let claim = LedgerEntry::InferenceJobClaim {
            job_id: "j1".into(),
            claimant: "stranger".into(), // neither worker nor requester
            evidence_hash: None,
            epoch: 1,
            nonce: 1,
            signed_by: "stranger".into(),
        };
        assert!(apply_claim(&chain, &claim).is_err());
    }

    // ── apply_review_vote ─────────────────────────────────────────────────────

    #[test]
    fn review_votes_resolve_majority() {
        let (chain, _dir) = make_chain("review");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j1", "bob", 0)).unwrap();
        apply_verify(&chain, &verify_entry("j1", "v1", "review_required", 0)).unwrap();
        let claim = LedgerEntry::InferenceJobClaim {
            job_id: "j1".into(), claimant: "bob".into(), evidence_hash: None,
            epoch: 1, nonce: 1, signed_by: "bob".into(),
        };
        apply_claim(&chain, &claim).unwrap();

        // Cast MIN_REVIEW_VOTES=3 approvals → Reviewed.
        for i in 0..3u64 {
            let vote = LedgerEntry::InferenceReviewVote {
                job_id: "j1".into(),
                reviewer: format!("reviewer{}", i),
                approved: true,
                epoch: 2,
                signed_by: format!("reviewer{}", i),
            };
            apply_review_vote(&chain, &vote).unwrap();
        }
        let job = get_job(&chain, "j1").unwrap();
        assert_eq!(job.status, JobStatus::Reviewed);
    }

    #[test]
    fn review_majority_rejection_moves_to_rejected() {
        let (chain, _dir) = make_chain("review_reject");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j1", "bob", 0)).unwrap();
        apply_verify(&chain, &verify_entry("j1", "v1", "review_required", 0)).unwrap();
        let claim = LedgerEntry::InferenceJobClaim {
            job_id: "j1".into(), claimant: "bob".into(), evidence_hash: None,
            epoch: 1, nonce: 1, signed_by: "bob".into(),
        };
        apply_claim(&chain, &claim).unwrap();

        // 2 rejections + 1 approval → majority reject.
        for (i, approved) in [(0u64, false), (1, false), (2, true)] {
            let vote = LedgerEntry::InferenceReviewVote {
                job_id: "j1".into(),
                reviewer: format!("r{}", i),
                approved,
                epoch: 2,
                signed_by: format!("r{}", i),
            };
            apply_review_vote(&chain, &vote).unwrap();
        }
        let job = get_job(&chain, "j1").unwrap();
        assert_eq!(job.status, JobStatus::Rejected);
    }

    #[test]
    fn review_duplicate_vote_fails() {
        let (chain, _dir) = make_chain("review_dup");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j1", "bob", 0)).unwrap();
        apply_verify(&chain, &verify_entry("j1", "v1", "review_required", 0)).unwrap();
        apply_claim(&chain, &LedgerEntry::InferenceJobClaim {
            job_id: "j1".into(), claimant: "bob".into(), evidence_hash: None,
            epoch: 1, nonce: 1, signed_by: "bob".into(),
        }).unwrap();

        let vote = LedgerEntry::InferenceReviewVote {
            job_id: "j1".into(), reviewer: "r1".into(), approved: true,
            epoch: 2, signed_by: "r1".into(),
        };
        apply_review_vote(&chain, &vote.clone()).unwrap();
        assert!(apply_review_vote(&chain, &vote).is_err());
    }

    // ── apply_pay (happy path) ────────────────────────────────────────────────

    #[test]
    fn pay_happy_path_distributes_correctly() {
        let (chain, _dir) = make_chain("pay_happy");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 1_000, "worker")).unwrap();
        apply_complete(&chain, &complete_entry("j1", "bob", 0)).unwrap();
        apply_verify(&chain, &verify_entry("j1", "v1", "approved", 0)).unwrap();

        let job = get_job(&chain, "j1").unwrap();
        let pay = build_pay_entry_happy(&chain, &job, 1).expect("build pay");
        apply_pay(&chain, &pay).expect("pay");

        let job = get_job(&chain, "j1").unwrap();
        assert_eq!(job.status, JobStatus::Paid);

        // Worker gets 80% of fee.
        let worker_balance = chain.store.get_balance("bob", NATIVE_TOKEN);
        let expected_worker = 1_000 * INFERENCE_FEE_WORKER_BPS / 10_000;
        assert_eq!(worker_balance, expected_worker, "worker gets 80%");
    }

    #[test]
    fn pay_fee_splits_sum_to_max_fee() {
        // Verify no tokens are created or destroyed.
        let fee = 10_000u64;
        let worker_amount = fee * INFERENCE_FEE_WORKER_BPS / 10_000;
        let verifier_pool = fee * INFERENCE_FEE_VERIFIER_BPS / 10_000;
        let recycle_amount = fee * INFERENCE_FEE_RECYCLE_BPS / 10_000;
        // Any remainder goes to refund.
        let paid = worker_amount + verifier_pool + recycle_amount;
        assert!(paid <= fee, "splits must not exceed fee");
        // At standard bps 8000+1500+500=10000, paid == fee exactly.
        assert_eq!(paid, fee);
    }

    // ── apply_cancel ──────────────────────────────────────────────────────────

    #[test]
    fn cancel_posted_job_refunds_requester() {
        let (chain, _dir) = make_chain("cancel");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();

        let cancel = LedgerEntry::InferenceJobCancel {
            job_id: "j1".into(),
            cancelled_by: "alice".into(),
            reason: "changed mind".into(),
            epoch: 0,
            nonce: 2,
            signed_by: "alice".into(),
        };
        apply_cancel(&chain, &cancel).expect("cancel");

        let job = get_job(&chain, "j1").unwrap();
        assert_eq!(job.status, JobStatus::Cancelled);
        // Refund: alice should have her 10_000 back.
        assert_eq!(chain.store.get_balance("alice", NATIVE_TOKEN), 10_000);
    }

    #[test]
    fn cancel_awarded_job_by_requester_fails() {
        let (chain, _dir) = make_chain("cancel_awarded");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();

        let cancel = LedgerEntry::InferenceJobCancel {
            job_id: "j1".into(),
            cancelled_by: "alice".into(),
            reason: "oops".into(),
            epoch: 0,
            nonce: 2,
            signed_by: "alice".into(),
        };
        assert!(apply_cancel(&chain, &cancel).is_err(), "requester cannot cancel awarded job");
    }

    #[test]
    fn cancel_awarded_by_system_succeeds() {
        let (chain, _dir) = make_chain("cancel_sys");
        seal(&chain, 0);
        fund(&chain, "alice", 10_000);
        apply_post(&chain, &post_entry("j1", "alice", 1_000, 0)).unwrap();
        apply_award(&chain, &award_entry("j1", "bob", 800, "worker")).unwrap();

        let cancel = LedgerEntry::InferenceJobCancel {
            job_id: "j1".into(),
            cancelled_by: "system".into(),
            reason: "deadline".into(),
            epoch: 0,
            nonce: 0,
            signed_by: "system".into(),
        };
        apply_cancel(&chain, &cancel).expect("system cancel");
        let job = get_job(&chain, "j1").unwrap();
        assert_eq!(job.status, JobStatus::Cancelled);
    }

    // ── build_pay_entry helpers ───────────────────────────────────────────────

    #[test]
    fn build_pay_happy_worker_gets_80_percent() {
        let (chain, _dir) = make_chain("build_pay");
        seal(&chain, 0);
        let fee = 10_000u64;
        let job = JobState {
            job_id: "j".into(),
            requester: "alice".into(),
            model: "m".into(),
            mode: "solo".into(),
            input_hash: "h".into(),
            max_fee: fee,
            min_reputation: 0,
            bid_window_epochs: 2,
            deadline_epoch: 10,
            posted_epoch: 0,
            status: JobStatus::Verified,
            winner: Some("bob".into()),
            awarded_fee: Some(fee),
            verifiers: vec!["v1".into()],
            result_hash: None,
            latency_ms: None,
            disputed_epoch: None,
            claimed_epoch: None,
            settled_epoch: None,
            completed_epoch: None,
            persist_on_fs: false,
            verdicts: vec![("v1".into(), "approved".into())],
            commits: vec![],
            min_verifiers: 1,
            node_fingerprint: None,
        };

        let pay = build_pay_entry_happy(&chain, &job, 1).unwrap();
        if let LedgerEntry::InferenceJobPay { worker_amount, verifier_payments, recycle_amount, .. } = pay {
            assert_eq!(worker_amount, 8_000);
            let ver_total: u64 = verifier_payments.iter().map(|(_, a)| a).sum();
            assert_eq!(ver_total, 1_500);
            assert_eq!(recycle_amount, 500);
        } else {
            panic!("expected InferenceJobPay");
        }
    }

    // ── select_best_bid ───────────────────────────────────────────────────────

    #[test]
    fn select_best_bid_picks_highest_rep_per_cost() {
        let (chain, _dir) = make_chain("select_bid");
        seal(&chain, 0);

        // Give bob higher rep.
        let mut rep = NodeReputation::new("bob");
        rep.score = 8_000;
        set_reputation(&chain, &rep).unwrap();

        let bids = vec![
            BidState { job_id: "j".into(), bidder: "alice".into(), fee: 500, role: "worker".into(), epoch: 0 },
            BidState { job_id: "j".into(), bidder: "bob".into(), fee: 500, role: "worker".into(), epoch: 0 },
        ];
        let best = select_best_bid(&bids, &chain, "worker").unwrap();
        assert_eq!(best.bidder, "bob", "higher rep wins at same fee");
    }

    #[test]
    fn select_best_bid_no_worker_bids_returns_none() {
        let (chain, _dir) = make_chain("select_none");
        seal(&chain, 0);
        let bids = vec![
            BidState { job_id: "j".into(), bidder: "v1".into(), fee: 100, role: "verifier".into(), epoch: 0 },
        ];
        assert!(select_best_bid(&bids, &chain, "worker").is_none());
    }

    // ── verifier approval rate ────────────────────────────────────────────────

    #[test]
    fn approval_rate_below_threshold_no_suspension() {
        let (chain, _dir) = make_chain("appr_ok");
        seal(&chain, 0);
        // 5 approvals + 5 rejections = 50% approval rate — below RUBBER_STAMP threshold.
        for _ in 0..5 {
            update_verifier_approval_rate_pub(&chain, "v1", true).unwrap();
        }
        for _ in 0..5 {
            update_verifier_approval_rate_pub(&chain, "v1", false).unwrap();
        }
        let vrep: serde_json::Value = chain.store
            .state_get("verifier_rep:v1")
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        assert!(!vrep["weight_halved"].as_bool().unwrap_or(false), "50% should not be penalised");
    }
}

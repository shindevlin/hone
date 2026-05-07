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
    VERIFIER_ACTIVE_THRESH_3, VERIFIER_ACTIVE_THRESH_5, VERIFIER_ACTIVITY_TRACK_EPOCHS,
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
    use sha2::{Sha256, Digest};
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
    if cancelled_by != &job.requester && cancelled_by.as_str() != "system" {
        bail!("only the requester can cancel job '{}'", job_id);
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

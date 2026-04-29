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
        max_fee, min_reputation, bid_window_epochs, deadline_epoch, epoch, ..
    } = entry else { bail!("wrong entry type") };

    if get_job(chain, job_id).is_some() {
        bail!("job '{}' already exists", job_id);
    }
    chain.store.debit(requester, NATIVE_TOKEN, *max_fee)?;
    // Escrow sits in recycle fund account until job is resolved.
    chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, *max_fee)?;

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
    })?;
    info!("inference job posted: {} model={} max_fee={}", job_id, model, max_fee);
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
    let LedgerEntry::InferenceJobComplete { job_id, worker, result_hash, latency_ms, .. } = entry
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

    let mut rep = get_reputation(chain, worker);
    rep.jobs_completed += 1;
    rep.total_latency_ms = rep.total_latency_ms.saturating_add(*latency_ms);
    rep.recompute_score();
    set_reputation(chain, &rep)?;
    set_job(chain, &job)?;
    Ok(())
}

pub fn apply_verify(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::InferenceJobVerify { job_id, verifier, verdict, epoch, .. } = entry
        else { bail!("wrong entry type") };

    let mut job = get_job(chain, job_id)
        .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;
    if job.status != JobStatus::Completed {
        bail!("job '{}' not in completed state for verification", job_id);
    }
    if !job.verifiers.contains(verifier) {
        bail!("'{}' is not an assigned verifier for job '{}'", verifier, job_id);
    }

    match verdict.as_str() {
        "approved" => {
            job.status = JobStatus::Verified;
        }
        "disputed" => {
            job.status = JobStatus::Disputed;
            job.disputed_epoch = Some(*epoch);
            // Mark verifier's failed judgement on worker reputation
            if let Some(ref winner) = job.winner.clone() {
                let mut rep = get_reputation(chain, winner);
                rep.jobs_failed += 1;
                rep.recompute_score();
                set_reputation(chain, &rep)?;
            }
        }
        other => bail!("unknown verdict '{}'", other),
    }
    set_job(chain, &job)?;
    Ok(())
}

pub fn apply_claim(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::InferenceJobClaim { job_id, claimant, epoch, .. } = entry
        else { bail!("wrong entry type") };

    let mut job = get_job(chain, job_id)
        .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;
    if job.status != JobStatus::Disputed {
        bail!("job '{}' is not in disputed state", job_id);
    }
    if job.winner.as_deref() != Some(claimant.as_str()) {
        bail!("only the awarded worker can claim job '{}'", job_id);
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
        recycle_amount, refund_amount, ..
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

    job.status = JobStatus::Paid;
    set_job(chain, &job)?;
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

/// Build InferenceJobPay for the happy path (Verified, no dispute).
pub fn build_pay_entry_happy(job: &JobState, verifiers: &[String], epoch: u64) -> Option<LedgerEntry> {
    let worker = job.winner.as_ref()?;
    let fee = job.awarded_fee?;

    let worker_amount = fee * INFERENCE_FEE_WORKER_BPS / 10_000;
    let verifier_pool = fee * INFERENCE_FEE_VERIFIER_BPS / 10_000;
    let recycle_amount = fee * INFERENCE_FEE_RECYCLE_BPS / 10_000;

    let n_ver = verifiers.len().max(1) as u64;
    let verifier_payments = verifiers.iter()
        .map(|v| (v.clone(), verifier_pool / n_ver))
        .collect::<Vec<_>>();

    let paid: u64 = worker_amount + verifier_payments.iter().map(|(_, a)| *a).sum::<u64>() + recycle_amount;
    let refund_amount = job.max_fee.saturating_sub(paid);

    Some(LedgerEntry::InferenceJobPay {
        job_id: job.job_id.clone(),
        worker: worker.clone(),
        worker_amount,
        verifier_payments,
        reviewer_payments: vec![],
        recycle_amount,
        refund_amount,
        epoch,
    })
}

/// Build InferenceJobPay for the disputed+reviewed path (worker wins review).
pub fn build_pay_entry_disputed(
    job: &JobState,
    verifiers: &[String],
    reviewers: &[String],
    epoch: u64,
) -> Option<LedgerEntry> {
    let worker = job.winner.as_ref()?;
    let fee = job.awarded_fee?;

    let worker_amount = fee * INFERENCE_FEE_WORKER_DISPUTED_BPS / 10_000;
    let verifier_pool = fee * INFERENCE_FEE_VERIFIER_DISPUTED_BPS / 10_000;
    let reviewer_pool = fee * INFERENCE_FEE_REVIEWER_BPS / 10_000;
    let recycle_amount = fee * INFERENCE_FEE_RECYCLE_DISPUTED_BPS / 10_000;

    let n_ver = verifiers.len().max(1) as u64;
    let n_rev = reviewers.len().max(1) as u64;
    let verifier_payments = verifiers.iter()
        .map(|v| (v.clone(), verifier_pool / n_ver))
        .collect::<Vec<_>>();
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
        epoch,
    })
}

/// Build InferenceJobPay for the NoFee / Rejected path (worker gets nothing).
/// Verifiers and reviewers still get paid; remainder refunded to requester.
pub fn build_pay_entry_nofee(
    job: &JobState,
    verifiers: &[String],
    reviewers: &[String],
    epoch: u64,
) -> LedgerEntry {
    let fee = job.awarded_fee.unwrap_or(job.max_fee);
    let verifier_pool = fee * INFERENCE_FEE_VERIFIER_DISPUTED_BPS / 10_000;
    let reviewer_pool = if reviewers.is_empty() { 0 } else { fee * INFERENCE_FEE_REVIEWER_BPS / 10_000 };
    let recycle_amount = fee * INFERENCE_FEE_RECYCLE_DISPUTED_BPS / 10_000;

    let n_ver = verifiers.len().max(1) as u64;
    let n_rev = reviewers.len().max(1) as u64;
    let verifier_payments = verifiers.iter()
        .map(|v| (v.clone(), verifier_pool / n_ver))
        .collect::<Vec<_>>();
    let reviewer_payments = reviewers.iter()
        .map(|r| (r.clone(), reviewer_pool / n_rev))
        .collect::<Vec<_>>();

    let paid: u64 = verifier_payments.iter().map(|(_, a)| *a).sum::<u64>()
        + reviewer_payments.iter().map(|(_, a)| *a).sum::<u64>()
        + recycle_amount;
    let refund_amount = job.max_fee.saturating_sub(paid);

    // On the NoFee path the worker gets 0 but we still record their account for
    // the ledger trail. Use the winner field if present, otherwise an empty
    // sentinel that signals "no worker was paid" in the audit log.
    let worker = job.winner.clone().unwrap_or_else(|| "__no_worker__".to_string());

    LedgerEntry::InferenceJobPay {
        job_id: job.job_id.clone(),
        worker,
        worker_amount: 0,
        verifier_payments,
        reviewer_payments,
        recycle_amount,
        refund_amount,
        epoch,
    }
}

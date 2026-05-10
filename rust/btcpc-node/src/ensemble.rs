//! Ensemble inference coordinator.
//!
//! Fans out a single job to N workers. Workers submit EnsembleVote entries with their
//! output hash. After the voting window, the majority-vote winner is paid and the
//! requester's escrow is released minus fees.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use btcpc_types::{LedgerEntry, NATIVE_TOKEN, RECYCLE_FUND_ACCOUNT};
use crate::chain::Chain;

const VOTE_WINDOW_EPOCHS: u64 = 3;
const MIN_ENSEMBLE_SIZE: u64 = 2;
const MAX_ENSEMBLE_SIZE: u64 = 7;
const WORKER_SHARE_BPS: u64 = 7_000; // 70% to workers
const RECYCLE_SHARE_BPS: u64 = 3_000; // 30% to recycle

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnsembleJob {
    pub job_id: String,
    pub requester: String,
    pub model: String,
    pub input_hash: String,
    pub max_fee: u64,
    pub n_workers: u64,
    pub posted_epoch: u64,
    pub vote_deadline: u64,
    pub votes: Vec<EnsembleVoteRecord>,
    pub status: EnsembleStatus,
    pub winner_output_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EnsembleStatus {
    Voting,
    Finalized,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnsembleVoteRecord {
    pub worker: String,
    pub output_hash: String,
    pub epoch: u64,
}

fn job_key(job_id: &str) -> String { format!("ensemble_job:{}", job_id) }

pub fn apply_job_post(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::EnsembleJobPost {
        job_id, requester, model, input_hash, max_fee, n_workers, epoch, ..
    } = entry else { bail!("wrong entry type") };

    if chain.store.state_get(&job_key(job_id)).is_some() {
        bail!("ensemble job '{}' already exists", job_id);
    }
    if *n_workers < MIN_ENSEMBLE_SIZE || *n_workers > MAX_ENSEMBLE_SIZE {
        bail!("n_workers must be {}-{}", MIN_ENSEMBLE_SIZE, MAX_ENSEMBLE_SIZE);
    }

    chain.store.debit(requester, NATIVE_TOKEN, *max_fee)?;

    let job = EnsembleJob {
        job_id: job_id.clone(),
        requester: requester.clone(),
        model: model.clone(),
        input_hash: input_hash.clone(),
        max_fee: *max_fee,
        n_workers: *n_workers,
        posted_epoch: *epoch,
        vote_deadline: epoch + VOTE_WINDOW_EPOCHS,
        votes: vec![],
        status: EnsembleStatus::Voting,
        winner_output_hash: None,
    };

    chain.store.state_set(&job_key(job_id), &serde_json::to_vec(&job)?)?;
    info!("[ensemble] job {} posted by {} ({} workers)", &job_id[..12], requester, n_workers);
    Ok(())
}

pub fn apply_vote(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::EnsembleVote {
        job_id, worker, output_hash, epoch, ..
    } = entry else { bail!("wrong entry type") };

    let raw = chain.store.state_get(&job_key(job_id))
        .ok_or_else(|| anyhow::anyhow!("ensemble job '{}' not found", job_id))?;
    let mut job: EnsembleJob = serde_json::from_slice(&raw)?;

    if job.status != EnsembleStatus::Voting {
        bail!("job '{}' is not accepting votes", job_id);
    }
    if *epoch > job.vote_deadline {
        bail!("vote window closed for job '{}'", job_id);
    }
    if job.votes.iter().any(|v| &v.worker == worker) {
        bail!("worker '{}' already voted on job '{}'", worker, job_id);
    }

    job.votes.push(EnsembleVoteRecord {
        worker: worker.clone(),
        output_hash: output_hash.clone(),
        epoch: *epoch,
    });

    // Check for majority once we have enough votes.
    if job.votes.len() as u64 >= (job.n_workers + 1) / 2 {
        if let Some(winner_hash) = majority_hash(&job.votes) {
            finalize_job(chain, &mut job, winner_hash)?;
        }
    }

    chain.store.state_set(&job_key(job_id), &serde_json::to_vec(&job)?)?;
    Ok(())
}

fn majority_hash(votes: &[EnsembleVoteRecord]) -> Option<String> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for v in votes { *counts.entry(&v.output_hash).or_default() += 1; }
    let threshold = (votes.len() / 2) + 1;
    counts.into_iter()
        .find(|(_, c)| *c >= threshold)
        .map(|(h, _)| h.to_owned())
}

fn finalize_job(chain: &Chain, job: &mut EnsembleJob, winner_hash: String) -> Result<()> {
    let winners: Vec<&str> = job.votes.iter()
        .filter(|v| v.output_hash == winner_hash)
        .map(|v| v.worker.as_str())
        .collect();

    let worker_pool = job.max_fee * WORKER_SHARE_BPS / 10_000;
    let recycle = job.max_fee.saturating_sub(worker_pool);
    let per_worker = worker_pool / winners.len().max(1) as u64;

    for w in &winners {
        chain.store.credit(w, NATIVE_TOKEN, per_worker)?;
    }
    if recycle > 0 {
        chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle)?;
    }

    job.status = EnsembleStatus::Finalized;
    job.winner_output_hash = Some(winner_hash);
    info!("[ensemble] job {} finalized — {} winner(s), {} dreams each",
        &job.job_id[..12], winners.len(), per_worker);
    Ok(())
}

/// Called at epoch seal: finalize any jobs whose vote window has expired.
pub fn sweep_expired(chain: &Chain, epoch: u64) {
    for (key, raw) in chain.store.state_scan_prefix("ensemble_job:") {
        let Ok(mut job) = serde_json::from_slice::<EnsembleJob>(&raw) else { continue };
        if job.status != EnsembleStatus::Voting { continue; }
        if epoch <= job.vote_deadline { continue; }

        if let Some(winner_hash) = majority_hash(&job.votes) {
            let _ = finalize_job(chain, &mut job, winner_hash);
        } else {
            // No majority — refund requester.
            job.status = EnsembleStatus::Cancelled;
            let _ = chain.store.credit(&job.requester, NATIVE_TOKEN, job.max_fee);
            info!("[ensemble] job {} expired with no majority — refunded", &job.job_id[..12]);
        }
        let _ = chain.store.state_set(&key, &serde_json::to_vec(&job).unwrap_or_default());
    }
}

//! Ensemble inference coordinator.
//!
//! Fans out a single job to N workers. Workers submit EnsembleVote entries with their
//! output hash. After the voting window, the majority-vote winner is paid and the
//! requester's escrow is released minus fees.
#![allow(dead_code)]

#![allow(dead_code)]
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use hone_types::{LedgerEntry, NATIVE_TOKEN, RECYCLE_FUND_ACCOUNT};
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
    info!("[ensemble] job {} finalized — {} winner(s), {} hunits each",
        &job.job_id[..12], winners.len(), per_worker);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hone_types::LedgerEntry;
    use tempfile::TempDir;

    fn make_chain() -> (Chain, TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(
            store, "test".to_string(), "hone-test".to_string(),
        );
        (chain, dir)
    }

    fn fund(chain: &Chain, account: &str, amount: u64) {
        chain.apply_entry(&LedgerEntry::AccountCreate {
            account: account.to_string(),
            keys: Default::default(),
            chain_proofs: vec![],
            epoch: 0,
            funded_by: None,
            machine_fingerprint: None,
        }).ok();
        chain.apply_entry(&LedgerEntry::GenesisAlloc {
            account: account.to_string(),
            amount,
            token: hone_types::NATIVE_TOKEN.to_string(),
        }).unwrap();
    }

    fn post_entry(job_id: &str, requester: &str, fee: u64, n_workers: u64) -> LedgerEntry {
        LedgerEntry::EnsembleJobPost {
            job_id: job_id.to_string(),
            requester: requester.to_string(),
            model: "qwen2.5:0.5b".to_string(),
            input_hash: "abc123".to_string(),
            max_fee: fee,
            n_workers,
            epoch: 1,
            nonce: 1,
            signed_by: requester.to_string(),
            signature: None,
        }
    }

    fn vote_entry(job_id: &str, worker: &str, output_hash: &str, epoch: u64) -> LedgerEntry {
        LedgerEntry::EnsembleVote {
            job_id: job_id.to_string(),
            worker: worker.to_string(),
            output_hash: output_hash.to_string(),
            epoch,
            nonce: 1,
            signed_by: worker.to_string(),
            signature: None,
        }
    }

    #[test]
    fn post_stores_job() {
        let (chain, _dir) = make_chain();
        fund(&chain, "requester", 1_000_000_000);

        apply_job_post(&chain, &post_entry("job0000000001", "requester", 100_000, 3)).unwrap();

        let raw = chain.store.state_get(&job_key("job0000000001")).unwrap();
        let job: EnsembleJob = serde_json::from_slice(&raw).unwrap();
        assert_eq!(job.job_id, "job0000000001");
        assert_eq!(job.status, EnsembleStatus::Voting);
        assert_eq!(job.n_workers, 3);
    }

    #[test]
    fn complete_marks_done_via_votes() {
        let (chain, _dir) = make_chain();
        fund(&chain, "requester", 1_000_000_000);
        fund(&chain, "w1", 0);
        fund(&chain, "w2", 0);
        fund(&chain, "w3", 0);

        // 3-worker job; majority threshold = (3+1)/2 = 2
        apply_job_post(&chain, &post_entry("job0000000002", "requester", 1_000_000, 3)).unwrap();

        // 2 workers agree on same hash → majority reached
        apply_vote(&chain, &vote_entry("job0000000002", "w1", "hash_abc", 2)).unwrap();
        apply_vote(&chain, &vote_entry("job0000000002", "w2", "hash_abc", 2)).unwrap();

        let raw = chain.store.state_get(&job_key("job0000000002")).unwrap();
        let job: EnsembleJob = serde_json::from_slice(&raw).unwrap();
        assert_eq!(job.status, EnsembleStatus::Finalized);
        assert_eq!(job.winner_output_hash, Some("hash_abc".to_string()));
    }

    #[test]
    fn complete_unknown_job_fails() {
        let (chain, _dir) = make_chain();
        let res = apply_vote(&chain, &vote_entry("nonexistent_job1", "worker", "hash", 2));
        assert!(res.is_err(), "voting on unknown job should fail");
    }

    #[test]
    fn post_requires_fee_escrowed() {
        let (chain, _dir) = make_chain();
        fund(&chain, "requester", 500_000);
        // max_fee=1_000_000, but requester only has 500_000
        let res = apply_job_post(&chain, &post_entry("job0000000003", "requester", 1_000_000, 2));
        assert!(res.is_err(), "posting with insufficient balance should fail");
    }
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

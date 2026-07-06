//! Fine-tune job lifecycle — LoRA job submission and completion.
//! P5-C: ~415 LOC

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::chain::Chain;
use hone_types::NATIVE_TOKEN;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FineTuneJobStatus {
    Open,
    Awarded,
    Complete,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FineTuneJob {
    pub job_id: String,
    pub requester: String,
    pub base_model: String,
    pub dataset_cid: String,
    pub max_fee: u64,
    pub status: FineTuneJobStatus,
    pub posted_epoch: u64,
    pub worker: Option<String>,
    pub adapter_cid: Option<String>,
    pub completed_epoch: Option<u64>,
}

fn job_key(job_id: &str) -> String { format!("finetune_job:{}", job_id) }

pub fn apply_post(
    chain: &Chain,
    job_id: &str,
    requester: &str,
    base_model: &str,
    dataset_cid: &str,
    max_fee: u64,
    epoch: u64,
) -> Result<()> {
    anyhow::ensure!(
        chain.store.state_get(&job_key(job_id)).is_none(),
        "fine-tune job '{}' already exists", job_id
    );
    anyhow::ensure!(max_fee > 0, "max_fee must be positive");
    // Escrow fee
    chain.store.debit(requester, NATIVE_TOKEN, max_fee)?;
    chain.store.state_set(&format!("finetune_escrow:{}", job_id), &max_fee.to_le_bytes())?;

    let job = FineTuneJob {
        job_id: job_id.to_string(),
        requester: requester.to_string(),
        base_model: base_model.to_string(),
        dataset_cid: dataset_cid.to_string(),
        max_fee,
        status: FineTuneJobStatus::Open,
        posted_epoch: epoch,
        worker: None,
        adapter_cid: None,
        completed_epoch: None,
    };
    chain.store.state_set(&job_key(job_id), &serde_json::to_vec(&job)?)?;
    Ok(())
}

pub fn apply_complete(
    chain: &Chain,
    job_id: &str,
    worker: &str,
    adapter_cid: &str,
    epoch: u64,
) -> Result<()> {
    let raw = chain.store.state_get(&job_key(job_id))
        .ok_or_else(|| anyhow::anyhow!("fine-tune job '{}' not found", job_id))?;
    let mut job: FineTuneJob = serde_json::from_slice(&raw)?;

    anyhow::ensure!(job.status == FineTuneJobStatus::Open, "job is not open");

    job.status = FineTuneJobStatus::Complete;
    job.worker = Some(worker.to_string());
    job.adapter_cid = Some(adapter_cid.to_string());
    job.completed_epoch = Some(epoch);

    // Release escrow to worker
    let escrow_raw = chain.store.state_get(&format!("finetune_escrow:{}", job_id))
        .unwrap_or_default();
    let escrow = if escrow_raw.len() == 8 {
        u64::from_le_bytes(escrow_raw.as_slice().try_into().unwrap_or([0u8; 8]))
    } else { 0 };
    if escrow > 0 {
        chain.store.credit(worker, NATIVE_TOKEN, escrow)?;
        chain.store.state_delete(&format!("finetune_escrow:{}", job_id))?;
    }

    chain.store.state_set(&job_key(job_id), &serde_json::to_vec(&job)?)?;
    Ok(())
}

pub fn get_job(chain: &Chain, job_id: &str) -> Option<FineTuneJob> {
    chain.store.state_get(&job_key(job_id))
        .and_then(|b| serde_json::from_slice(&b).ok())
}

pub fn list_open_jobs(chain: &Chain) -> Vec<FineTuneJob> {
    chain.store.state_scan_prefix("finetune_job:")
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<FineTuneJob>(&v).ok())
        .filter(|j| j.status == FineTuneJobStatus::Open)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chain(label: &str) -> (Chain, tempfile::TempDir) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("hone_test_{}_", label))
            .tempdir()
            .unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(store, format!("node-{}", label), "hone-test".to_string());
        (chain, dir)
    }

    fn fund(chain: &Chain, account: &str, amount: u64) {
        chain.apply_entry(&hone_types::LedgerEntry::GenesisAlloc {
            account: account.to_string(),
            amount,
            token: hone_types::NATIVE_TOKEN.to_string(),
        }).unwrap();
    }

    #[test]
    fn test_post_stores_job_with_status_open() {
        let (chain, _dir) = make_chain("ft_post");
        fund(&chain, "alice", 1_000_000);
        apply_post(&chain, "job1", "alice", "llama3", "bafycid1", 500, 1).unwrap();
        let job = get_job(&chain, "job1").expect("job should exist");
        assert_eq!(job.status, FineTuneJobStatus::Open);
        assert_eq!(job.requester, "alice");
        assert_eq!(job.base_model, "llama3");
    }

    #[test]
    fn test_complete_updates_status_and_adapter_cid() {
        let (chain, _dir) = make_chain("ft_complete");
        fund(&chain, "alice", 1_000_000);
        apply_post(&chain, "job2", "alice", "llama3", "bafycid2", 500, 1).unwrap();
        apply_complete(&chain, "job2", "worker1", "bafyadapter1", 2).unwrap();
        let job = get_job(&chain, "job2").expect("job should exist");
        assert_eq!(job.status, FineTuneJobStatus::Complete);
        assert_eq!(job.adapter_cid, Some("bafyadapter1".to_string()));
        assert_eq!(job.worker, Some("worker1".to_string()));
    }

    #[test]
    fn test_complete_on_unknown_job_returns_err() {
        let (chain, _dir) = make_chain("ft_unknown");
        let result = apply_complete(&chain, "nonexistent-job", "worker1", "bafyadapter1", 1);
        assert!(result.is_err());
    }
}

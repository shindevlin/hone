//! Computer-use automation jobs — Playwright task submission and completion.
//! P5-D: ~510 LOC

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::chain::Chain;
use hone_types::NATIVE_TOKEN;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ComputerUseJobStatus {
    Open,
    Awarded,
    Complete,
    Expired,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputerUseJob {
    pub job_id: String,
    pub requester: String,
    /// Serialized JSON task: { url, actions: [...], expected_outcome }
    pub task_json: String,
    pub max_fee: u64,
    pub status: ComputerUseJobStatus,
    pub posted_epoch: u64,
    pub worker: Option<String>,
    pub result_cid: Option<String>,
    pub completed_epoch: Option<u64>,
}

fn job_key(job_id: &str) -> String { format!("computer_use_job:{}", job_id) }

pub fn apply_post(
    chain: &Chain,
    job_id: &str,
    requester: &str,
    task_json: &str,
    max_fee: u64,
    epoch: u64,
) -> Result<()> {
    anyhow::ensure!(
        chain.store.state_get(&job_key(job_id)).is_none(),
        "computer-use job '{}' already exists", job_id
    );
    anyhow::ensure!(max_fee > 0, "max_fee must be positive");
    // Validate task_json is parseable
    serde_json::from_str::<serde_json::Value>(task_json)
        .map_err(|e| anyhow::anyhow!("invalid task_json: {}", e))?;

    // Escrow fee
    chain.store.debit(requester, NATIVE_TOKEN, max_fee)?;
    chain.store.state_set(&format!("computer_use_escrow:{}", job_id), &max_fee.to_le_bytes())?;

    let job = ComputerUseJob {
        job_id: job_id.to_string(),
        requester: requester.to_string(),
        task_json: task_json.to_string(),
        max_fee,
        status: ComputerUseJobStatus::Open,
        posted_epoch: epoch,
        worker: None,
        result_cid: None,
        completed_epoch: None,
    };
    chain.store.state_set(&job_key(job_id), &serde_json::to_vec(&job)?)?;
    Ok(())
}

pub fn apply_complete(
    chain: &Chain,
    job_id: &str,
    worker: &str,
    result_cid: &str,
    epoch: u64,
) -> Result<()> {
    let raw = chain.store.state_get(&job_key(job_id))
        .ok_or_else(|| anyhow::anyhow!("computer-use job '{}' not found", job_id))?;
    let mut job: ComputerUseJob = serde_json::from_slice(&raw)?;

    anyhow::ensure!(job.status == ComputerUseJobStatus::Open, "job is not open");

    job.status = ComputerUseJobStatus::Complete;
    job.worker = Some(worker.to_string());
    job.result_cid = Some(result_cid.to_string());
    job.completed_epoch = Some(epoch);

    // Release escrow to worker
    let escrow_raw = chain.store.state_get(&format!("computer_use_escrow:{}", job_id))
        .unwrap_or_default();
    let escrow = if escrow_raw.len() == 8 {
        u64::from_le_bytes(escrow_raw.as_slice().try_into().unwrap_or([0u8; 8]))
    } else { 0 };
    if escrow > 0 {
        chain.store.credit(worker, NATIVE_TOKEN, escrow)?;
        chain.store.state_delete(&format!("computer_use_escrow:{}", job_id))?;
    }

    chain.store.state_set(&job_key(job_id), &serde_json::to_vec(&job)?)?;
    Ok(())
}

pub fn get_job(chain: &Chain, job_id: &str) -> Option<ComputerUseJob> {
    chain.store.state_get(&job_key(job_id))
        .and_then(|b| serde_json::from_slice(&b).ok())
}

pub fn list_open_jobs(chain: &Chain) -> Vec<ComputerUseJob> {
    chain.store.state_scan_prefix("computer_use_job:")
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<ComputerUseJob>(&v).ok())
        .filter(|j| j.status == ComputerUseJobStatus::Open)
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

    const TASK: &str = r#"{"url":"https://example.com","actions":[],"expected_outcome":"done"}"#;

    #[test]
    fn test_post_stores_job_with_status_open() {
        let (chain, _dir) = make_chain("cu_post");
        fund(&chain, "alice", 1_000_000);
        apply_post(&chain, "cujob1", "alice", TASK, 500, 1).unwrap();
        let job = get_job(&chain, "cujob1").expect("job should exist");
        assert_eq!(job.status, ComputerUseJobStatus::Open);
        assert_eq!(job.requester, "alice");
    }

    #[test]
    fn test_complete_updates_status_and_result_cid() {
        let (chain, _dir) = make_chain("cu_complete");
        fund(&chain, "alice", 1_000_000);
        apply_post(&chain, "cujob2", "alice", TASK, 500, 1).unwrap();
        apply_complete(&chain, "cujob2", "worker1", "bafyresult1", 2).unwrap();
        let job = get_job(&chain, "cujob2").expect("job should exist");
        assert_eq!(job.status, ComputerUseJobStatus::Complete);
        assert_eq!(job.result_cid, Some("bafyresult1".to_string()));
        assert_eq!(job.worker, Some("worker1".to_string()));
    }

    #[test]
    fn test_complete_on_unknown_job_returns_err() {
        let (chain, _dir) = make_chain("cu_unknown");
        let result = apply_complete(&chain, "nonexistent-job", "worker1", "bafyresult1", 1);
        assert!(result.is_err());
    }
}

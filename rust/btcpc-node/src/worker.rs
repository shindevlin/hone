//! Inference job worker daemon.
//!
//! Scans the chain for Posted jobs, bids as a worker, executes the job via
//! Ollama when awarded, and submits InferenceJobComplete with the result hash.
//!
//! This is the correct inference participation mechanism. miner.rs (direct
//! Mine entries) is the old self-directed approach and does not interact with
//! the marketplace.
//!
//! Env vars:
//!   BTCPC_WORKER      — "true" to activate this daemon
//!   OLLAMA_URL        — Ollama base URL (default http://localhost:11434)
//!   BTCPC_MODEL       — default model hint; job specifies model to use
//!   BTCPC_WORKER_FEE_BPS — fraction of max_fee to bid (default 9000 = 90%)

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tracing::{info, warn};
use btcpc_types::LedgerEntry;

use crate::chain::Chain;
use crate::inference::{self, JobStatus};
use crate::net::NetCmd;
use crate::udp_gossip::UdpGossipHandle;
use crate::utils::now_ms;

pub async fn run_worker(
    chain: Arc<Chain>,
    account: String,
    cmd_tx: tokio::sync::mpsc::Sender<NetCmd>,
    udp: UdpGossipHandle,
) {
    info!("worker daemon started: account={}", account);

    // Track jobs we've already completed to avoid re-submitting.
    let mut completed: HashSet<String> = HashSet::new();

    loop {
        let epoch = chain.current_epoch();

        bid_on_posted_jobs(&chain, &account, epoch, &cmd_tx, &udp).await;
        execute_awarded_jobs(&chain, &account, epoch, &cmd_tx, &mut completed).await;

        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

async fn bid_on_posted_jobs(
    chain: &Chain,
    account: &str,
    epoch: u64,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
    udp: &UdpGossipHandle,
) {
    let fee_bps: u64 = std::env::var("BTCPC_WORKER_FEE_BPS")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(9000);

    let posted_jobs: Vec<inference::JobState> = chain.store
        .state_scan_prefix("infer_job:")
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice(&v).ok())
        .filter(|j: &inference::JobState| {
            j.status == JobStatus::Posted
                && epoch < j.posted_epoch + j.bid_window_epochs
        })
        .collect();

    for job in posted_jobs {
        // Skip if already bid on this job.
        let bid_key = format!("infer_bid:{}:{}", job.job_id, account);
        if chain.store.state_get(&bid_key).is_some() {
            continue;
        }

        // Check reputation gating.
        let rep = inference::get_reputation(chain, account);
        if rep.score < job.min_reputation {
            continue;
        }

        let bid_fee = job.max_fee.saturating_mul(fee_bps) / 10_000;

        let entry = LedgerEntry::InferenceJobBid {
            job_id:     job.job_id.clone(),
            bidder:     account.to_owned(),
            fee:        bid_fee,
            role:       "worker".to_owned(),
            epoch,
            nonce:      now_ms(),
            signed_by:  account.to_owned(),
        };

        match chain.apply_entry(&entry) {
            Ok(_) => {
                broadcast(&entry, cmd_tx).await;
                udp.send_if_light(entry);
                info!("worker: bid on job {} fee={} model={}", job.job_id, bid_fee, job.model);
            }
            Err(e) => warn!("worker: bid failed for job {}: {}", job.job_id, e),
        }
    }
}

async fn execute_awarded_jobs(
    chain: &Chain,
    account: &str,
    epoch: u64,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
    completed: &mut HashSet<String>,
) {
    let awarded_jobs: Vec<inference::JobState> = chain.store
        .state_scan_prefix("infer_job:")
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice(&v).ok())
        .filter(|j: &inference::JobState| {
            j.status == JobStatus::Awarded
                && j.winner.as_deref() == Some(account)
        })
        .collect();

    for job in awarded_jobs {
        if completed.contains(&job.job_id) {
            continue;
        }

        // Retrieve input text from chain state (stored at job post time).
        let input_text = chain.store
            .state_get(&format!("infer_input:{}", job.job_id))
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();

        if input_text.is_empty() {
            warn!("worker: no input text for job {} — skipping", job.job_id);
            completed.insert(job.job_id.clone());
            continue;
        }

        info!("worker: executing job {} model={}", job.job_id, job.model);

        let (result_hash, output_text, latency_ms) =
            call_ollama(&job.model, &input_text).await;

        if result_hash.is_empty() {
            warn!("worker: Ollama returned empty result for job {}", job.job_id);
            continue;
        }

        // Store output for verifiers before submitting the completion entry.
        let _ = chain.store.state_set(
            &format!("infer_output:{}", job.job_id),
            output_text.as_bytes(),
        );

        let entry = LedgerEntry::InferenceJobComplete {
            job_id:      job.job_id.clone(),
            worker:      account.to_owned(),
            result_hash: result_hash.clone(),
            latency_ms,
            epoch,
            signed_by:   account.to_owned(),
        };

        match chain.apply_entry(&entry) {
            Ok(_) => {
                broadcast(&entry, cmd_tx).await;
                completed.insert(job.job_id.clone());
                info!("worker: job {} complete — {}ms result={}", job.job_id, latency_ms, &result_hash[..8]);
            }
            Err(e) => warn!("worker: complete failed for job {}: {}", job.job_id, e),
        }
    }
}

async fn call_ollama(model: &str, prompt: &str) -> (String, String, u64) {
    let ollama_url = std::env::var("OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned());

    let start = std::time::Instant::now();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/generate", ollama_url))
        .timeout(Duration::from_secs(120))
        .json(&serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "keep_alive": "0s",
            "options": { "temperature": 0.0 },
        }))
        .send()
        .await;

    let latency_ms = start.elapsed().as_millis() as u64;

    match resp {
        Ok(r) if r.status().is_success() => {
            if let Ok(body) = r.json::<serde_json::Value>().await {
                let text = body["response"].as_str().unwrap_or("").to_owned();
                let hash = hex::encode(Sha256::digest(text.as_bytes()));
                return (hash, text, latency_ms);
            }
        }
        Ok(r) => warn!("worker: Ollama {} ({}ms)", r.status(), latency_ms),
        Err(e) => warn!("worker: Ollama unreachable: {} ({}ms)", e, latency_ms),
    }

    (String::new(), String::new(), latency_ms)
}

async fn broadcast(entry: &LedgerEntry, cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>) {
    let envelope = serde_json::json!({ "entry": entry });
    if let Ok(data) = serde_json::to_vec(&envelope) {
        let _ = cmd_tx.send(NetCmd::Broadcast {
            topic: "btcpc/entries",
            data,
        }).await;
    }
}

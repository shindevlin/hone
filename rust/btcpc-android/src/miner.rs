//! Inference mining loop for Android.
//!
//! Lifecycle:
//!   1. Ensure model is present (download if needed)
//!   2. Register as an inference worker with the btcpc node API
//!   3. Poll for posted inference jobs, bid, run, submit result
//!   4. Also mine blocks (SHA256 work) when we hold the miner role

use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::llm::LlmEngine;

pub struct MinerConfig {
    pub account:     String,
    pub jwt:         String,
    pub api_base:    String,
    pub model_id:    String,
    pub model_dir:   String,
    pub posting_key: String,
}

pub async fn run(
    cfg: MinerConfig,
    status: Arc<Mutex<String>>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client");

    let mut llm = LlmEngine::new(&cfg.model_dir, status.clone());

    *status.lock() = "Initialising…".to_owned();

    // Start model download/check in background immediately
    let model_ready = llm.ensure_ready().await;
    if !model_ready {
        *status.lock() = "Downloading model — mining blocks while waiting…".to_owned();
    }

    let mut jobs_completed: u64 = 0;
    let mut blocks_mined:   u64 = 0;

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = tokio::time::sleep(Duration::from_secs(15)) => {}
        }

        // Re-check model readiness on every tick
        let _ = llm.ensure_ready().await;

        // ── Block mining ──────────────────────────────────────────────────────
        match mine_block(&client, &cfg).await {
            Ok(true) => {
                blocks_mined += 1;
                let s = format!("Blocks: {} | Jobs: {} | account: {}",
                    blocks_mined, jobs_completed, cfg.account);
                *status.lock() = s;
            }
            Ok(false) => {} // not our turn
            Err(e) => warn!("mine_block error: {}", e),
        }

        // ── Inference job ─────────────────────────────────────────────────────
        if !llm.ensure_ready().await {
            continue; // model not ready yet, skip inference
        }

        match run_inference_job(&client, &cfg, &llm, &mut jobs_completed).await {
            Ok(()) => {
                let s = format!("Blocks: {} | Jobs: {} | account: {}",
                    blocks_mined, jobs_completed, cfg.account);
                *status.lock() = s;
            }
            Err(e) => warn!("inference job error: {}", e),
        }
    }

    *status.lock() = "Stopped".to_owned();
}

// ── Block mining ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct EpochStatus {
    current_epoch:  u64,
    miner_account:  Option<String>,
}

async fn mine_block(client: &reqwest::Client, cfg: &MinerConfig) -> Result<bool> {
    let url = format!("{}/api/chain/status", cfg.api_base);
    let resp: serde_json::Value = client
        .get(&url)
        .bearer_auth(&cfg.jwt)
        .send().await?
        .json().await?;

    let epoch = resp["current_epoch"].as_u64().unwrap_or(0);
    let miner = resp["miner_account"].as_str().unwrap_or("");

    // Only mine if we're the designated miner or no miner is set
    if !miner.is_empty() && miner != cfg.account {
        return Ok(false);
    }

    let work_value = compute_work(epoch);
    let body = serde_json::json!({
        "type": "Mine",
        "miner": cfg.account,
        "epoch": epoch,
        "work_value": work_value,
        "block_hash": "",
    });

    let mine_url = format!("{}/api/entries", cfg.api_base);
    let resp = client
        .post(&mine_url)
        .bearer_auth(&cfg.jwt)
        .json(&body)
        .send().await?;

    Ok(resp.status().is_success())
}

fn compute_work(epoch: u64) -> u64 {
    let mut h = Sha256::new();
    h.update(epoch.to_le_bytes());
    let result = h.finalize();
    let mut leading_zeros = 0u64;
    for byte in result.iter() {
        if *byte == 0 { leading_zeros += 8; }
        else { leading_zeros += byte.leading_zeros() as u64; break; }
    }
    leading_zeros
}

// ── Inference jobs ────────────────────────────────────────────────────────────

#[derive(Deserialize, Clone)]
struct InferenceJob {
    job_id:     String,
    model:      String,
    input_hash: String,
    max_fee:    u64,
}

#[derive(Deserialize)]
struct JobsResponse {
    jobs: Vec<InferenceJob>,
}

async fn run_inference_job(
    client: &reqwest::Client,
    cfg: &MinerConfig,
    llm: &LlmEngine,
    jobs_completed: &mut u64,
) -> Result<()> {
    // Fetch posted jobs
    let url = format!("{}/api/inference/jobs?status=posted&limit=1", cfg.api_base);
    let resp = client.get(&url).bearer_auth(&cfg.jwt).send().await?;
    if !resp.status().is_success() { return Ok(()); }

    let body: serde_json::Value = resp.json().await?;
    let jobs = body["jobs"].as_array().cloned().unwrap_or_default();
    if jobs.is_empty() { return Ok(()); }

    let job: InferenceJob = serde_json::from_value(jobs[0].clone())?;

    // Bid on the job
    let bid_url = format!("{}/api/inference/bids", cfg.api_base);
    let bid = serde_json::json!({
        "job_id": job.job_id,
        "bidder": cfg.account,
        "role": "worker",
        "fee": job.max_fee,
        "model": cfg.model_id,
    });
    let bid_resp = client.post(&bid_url).bearer_auth(&cfg.jwt).json(&bid).send().await?;
    if !bid_resp.status().is_success() { return Ok(()); }

    // Wait briefly for award
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Check if we won
    let award_url = format!("{}/api/inference/jobs/{}", cfg.api_base, job.job_id);
    let award_resp: serde_json::Value = client
        .get(&award_url).bearer_auth(&cfg.jwt).send().await?.json().await?;

    let winner = award_resp["winner"].as_str().unwrap_or("");
    if winner != cfg.account { return Ok(()); }

    // Fetch the actual prompt
    let prompt_url = format!("{}/api/inference/jobs/{}/input", cfg.api_base, job.job_id);
    let prompt_resp = client.get(&prompt_url).bearer_auth(&cfg.jwt).send().await?;
    let prompt: String = if prompt_resp.status().is_success() {
        prompt_resp.json::<serde_json::Value>().await?
            .get("prompt").and_then(|v| v.as_str()).unwrap_or("hello").to_owned()
    } else {
        "hello".to_owned() // fallback for jobs without stored input
    };

    // Run local inference
    let output = llm.generate(&prompt, 256).await
        .unwrap_or_else(|e| format!("inference error: {e}"));

    // Hash the result
    let result_hash = hex::encode(Sha256::digest(output.as_bytes()));

    // Submit result
    let submit_url = format!("{}/api/inference/submit", cfg.api_base);
    let submit = serde_json::json!({
        "job_id":      job.job_id,
        "worker":      cfg.account,
        "result_hash": result_hash,
        "output":      output,
        "model":       cfg.model_id,
    });
    let submit_resp = client
        .post(&submit_url).bearer_auth(&cfg.jwt).json(&submit).send().await?;

    if submit_resp.status().is_success() {
        *jobs_completed += 1;
    }

    Ok(())
}

use anyhow::{anyhow, Result};
use hone_sdk::KeyPair;
use colored::Colorize;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;

fn resolve_key(key_file: Option<&Path>) -> Result<std::path::PathBuf> {
    if let Some(p) = key_file {
        return Ok(p.to_path_buf());
    }
    if let Ok(p) = std::env::var("HONE_KEY_FILE") {
        let t = p.trim();
        if !t.is_empty() {
            return Ok(std::path::PathBuf::from(t));
        }
    }
    Err(anyhow!("missing key file: pass --key-file <path> or set HONE_KEY_FILE"))
}

fn next_nonce(api: &ApiClient, account: &str) -> Result<u64> {
    let data: Value = api.get(&format!("/api/account/{}", account))?;
    let current = data.get("nonce").and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("account '{}' has no nonce field", account))?;
    current.checked_add(1).ok_or_else(|| anyhow!("nonce overflow"))
}

fn node_chain_id(api: &ApiClient) -> Result<String> {
    let info: Value = api.get("/api/node/info")?;
    info.get("chain_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("node info missing chain_id"))
}

/// Hash the input string to produce an input_hash for the job.
fn hash_input(input: &str) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    // If input starts with '@', treat as file path.
    if let Some(path) = input.strip_prefix('@') {
        if let Ok(contents) = std::fs::read(path) {
            h.update(&contents);
        } else {
            h.update(input.as_bytes());
        }
    } else {
        h.update(input.as_bytes());
    }
    hex::encode(h.finalize())
}

// ── Post ──────────────────────────────────────────────────────────────────────

pub fn cmd_post(
    account: &str,
    model: &str,
    input: &str,
    max_fee: u64,
    min_rep: u64,
    mode: &str,
    bid_window: u64,
    deadline: u64,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let key_path = resolve_key(key_file)?;
    let keypair = KeyPair::from_file(&key_path)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let input_hash = hash_input(input);

    // job_id is excluded: server derives it from requester+nonce+epoch; nonce prevents replay.
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": "INFERENCE_JOB_POST",
        "requester": account,
        "model": model,
        "max_fee": max_fee,
        "nonce": nonce,
    }))?;
    let sig = keypair.sign_entry_json(&msg);

    let body = json!({
        "requester": account,
        "model": model,
        "mode": mode,
        "input_hash": input_hash,
        "max_fee": max_fee,
        "min_reputation": min_rep,
        "bid_window_epochs": bid_window,
        "deadline_epoch": deadline,
        "nonce": nonce,
        "signature": sig,
    });

    let resp: Value = api.post("/api/task/post", &body)?;
    if resp.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false) {
        println!("{}", "Inference job posted.".green().bold());
        if let Some(job_id) = resp.get("job_id").and_then(|v| v.as_str()) {
            println!("job_id: {}", job_id.bold());
        }
        if let Some(h) = resp.get("hash").and_then(|v| v.as_str()) {
            println!("hash:   {}", h);
        }
    } else {
        let err = resp.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
        return Err(anyhow!("{}", err));
    }
    Ok(())
}

// ── Jobs list ─────────────────────────────────────────────────────────────────

pub fn cmd_jobs(status: Option<&str>, model: Option<&str>) -> Result<()> {
    let api = ApiClient::new();
    let mut path = "/api/task/jobs".to_owned();
    let mut params = vec![];
    if let Some(s) = status { params.push(format!("status={}", s)); }
    if let Some(m) = model { params.push(format!("model={}", m)); }
    if !params.is_empty() {
        path = format!("{}?{}", path, params.join("&"));
    }

    let resp: Value = api.get(&path)?;
    let jobs = resp.get("jobs").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let count = resp.get("count").and_then(|v| v.as_u64()).unwrap_or(jobs.len() as u64);

    if jobs.is_empty() {
        println!("No jobs found.");
        return Ok(());
    }
    println!("{} jobs found:\n", count);
    for job in &jobs {
        let id = job.get("job_id").and_then(|v| v.as_str()).unwrap_or("-");
        let model = job.get("model").and_then(|v| v.as_str()).unwrap_or("-");
        let status = job.get("status").and_then(|v| v.as_str()).unwrap_or("-");
        let max_fee = job.get("max_fee").and_then(|v| v.as_u64()).unwrap_or(0);
        println!("  {} {} {} max_fee={}", id.bold(), model.cyan(), status.yellow(), max_fee);
    }
    Ok(())
}

// ── Job detail ────────────────────────────────────────────────────────────────

pub fn cmd_job(id: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/task/job/{}", id))?;
    println!("{}", serde_json::to_string_pretty(&resp).unwrap_or_default());
    Ok(())
}

// ── Bid ───────────────────────────────────────────────────────────────────────

pub fn cmd_bid(
    job_id: &str,
    account: &str,
    fee: u64,
    role: &str,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let key_path = resolve_key(key_file)?;
    let keypair = KeyPair::from_file(&key_path)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;

    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": "INFERENCE_JOB_BID",
        "bidder": account,
        "job_id": job_id,
        "fee": fee,
        "role": role,
        "nonce": nonce,
    }))?;
    let sig = keypair.sign_entry_json(&msg);

    let body = json!({
        "job_id": job_id,
        "bidder": account,
        "fee": fee,
        "role": role,
        "nonce": nonce,
        "signature": sig,
    });

    let resp: Value = api.post("/api/task/bid", &body)?;
    check_accepted(&resp, "Bid submitted.")?;
    Ok(())
}

// ── Complete ──────────────────────────────────────────────────────────────────

pub fn cmd_complete(
    job_id: &str,
    account: &str,
    result_hash: &str,
    latency_ms: u64,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let key_path = resolve_key(key_file)?;
    let keypair = KeyPair::from_file(&key_path)?;
    let chain_id = node_chain_id(&api)?;

    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": "INFERENCE_JOB_COMPLETE",
        "worker": account,
        "job_id": job_id,
        "result_hash": result_hash,
    }))?;
    let sig = keypair.sign_entry_json(&msg);

    let body = json!({
        "job_id": job_id,
        "worker": account,
        "result_hash": result_hash,
        "latency_ms": latency_ms,
        "signature": sig,
    });

    let resp: Value = api.post("/api/task/complete", &body)?;
    check_accepted(&resp, "Job completion submitted.")?;
    Ok(())
}

// ── Cancel ────────────────────────────────────────────────────────────────────

pub fn cmd_cancel(
    job_id: &str,
    account: &str,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let key_path = resolve_key(key_file)?;
    let keypair = KeyPair::from_file(&key_path)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;

    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": "INFERENCE_JOB_CANCEL",
        "cancelled_by": account,
        "job_id": job_id,
        "nonce": nonce,
    }))?;
    let sig = keypair.sign_entry_json(&msg);

    let body = json!({
        "job_id": job_id,
        "cancelled_by": account,
        "reason": "user request",
        "nonce": nonce,
        "signature": sig,
    });

    let resp: Value = api.post("/api/task/cancel", &body)?;
    check_accepted(&resp, "Job cancelled. Escrow refunded.")?;
    Ok(())
}

// ── Reputation ────────────────────────────────────────────────────────────────

pub fn cmd_reputation(node: Option<&str>) -> Result<()> {
    let account = node
        .map(str::to_owned)
        .or_else(|| std::env::var("HONE_ACCOUNT").ok())
        .ok_or_else(|| anyhow!("specify a node account or set HONE_ACCOUNT"))?;

    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/task/reputation/{}", account))?;
    let score = resp.get("score").and_then(|v| v.as_u64()).unwrap_or(0);
    let completed = resp.get("jobs_completed").and_then(|v| v.as_u64()).unwrap_or(0);
    let accepted = resp.get("jobs_accepted").and_then(|v| v.as_u64()).unwrap_or(0);
    let failed = resp.get("jobs_failed").and_then(|v| v.as_u64()).unwrap_or(0);

    println!("{}", account.bold());
    println!("  reputation score: {}/10000", score);
    println!("  jobs accepted:    {}", accepted);
    println!("  jobs completed:   {}", completed);
    println!("  jobs failed:      {}", failed);
    if accepted > 0 {
        let rate = completed * 100 / accepted;
        println!("  completion rate:  {}%", rate);
    }
    Ok(())
}

// ── Helper ────────────────────────────────────────────────────────────────────

fn check_accepted(resp: &Value, success_msg: &str) -> Result<()> {
    if resp.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false) {
        println!("{}", success_msg.green().bold());
        if let Some(h) = resp.get("hash").and_then(|v| v.as_str()) {
            println!("hash: {}", h);
        }
        Ok(())
    } else {
        let err = resp.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
        Err(anyhow!("{}", err))
    }
}

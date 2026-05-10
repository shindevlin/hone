use anyhow::{anyhow, Result};
use colored::Colorize;
use serde_json::{json, Value};

use crate::api::ApiClient;

fn read_input(input: &str) -> Vec<u8> {
    if let Some(path) = input.strip_prefix('@') {
        std::fs::read(path).unwrap_or_else(|_| input.as_bytes().to_vec())
    } else {
        input.as_bytes().to_vec()
    }
}

// ── Create ────────────────────────────────────────────────────────────────────

pub fn cmd_science_create(
    account: &str,
    title: &str,
    job_type: &str,
    input: &str,
    max_fee: u64,
    open_source: bool,
    model: Option<&str>,
) -> Result<()> {
    let api = ApiClient::new();
    let input_bytes = read_input(input);

    let body = json!({
        "requester": account,
        "title": title,
        "job_type": job_type,
        "input_b64": base64_encode(&input_bytes),
        "max_fee": max_fee,
        "open_source": open_source,
        "model": model,
    });

    let res: Value = api.post("/api/science/jobs", &body)?;
    let job_id = res.get("job_id").and_then(|v| v.as_str()).unwrap_or("?");
    let fee_paid = res.get("fee_paid").and_then(|v| v.as_u64()).unwrap_or(0);
    let oss = res.get("open_source").and_then(|v| v.as_bool()).unwrap_or(false);

    println!("{}", "Scientific job created.".green().bold());
    println!("{} {}", "Job ID:".bold(), job_id);
    println!("{} {} dreams{}", "Fee paid:".bold(), fee_paid,
        if oss { " (40% open-source discount applied)" } else { "" });
    println!("{} {}", "Status:".bold(), res.get("status").and_then(|v| v.as_str()).unwrap_or("queued"));
    Ok(())
}

// ── List ──────────────────────────────────────────────────────────────────────

pub fn cmd_science_jobs(job_type: Option<&str>) -> Result<()> {
    let api = ApiClient::new();
    let path = match job_type {
        Some(t) => format!("/api/science/jobs/type/{}", t),
        None => "/api/science/jobs".to_owned(),
    };
    let jobs: Value = api.get(&path)?;
    let arr = jobs.as_array().ok_or_else(|| anyhow!("unexpected response"))?;

    if arr.is_empty() {
        println!("{}", "No science jobs found.".yellow());
        return Ok(());
    }

    println!("{:<36}  {:<16}  {:<10}  {}", "JOB ID", "TYPE", "STATUS", "TITLE");
    println!("{}", "─".repeat(80));
    for j in arr {
        let id = j.get("job_id").and_then(|v| v.as_str()).unwrap_or("?");
        let jtype = j.get("job_type").and_then(|v| v.as_str()).unwrap_or("?");
        let status = j.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        let title = j.get("title").and_then(|v| v.as_str()).unwrap_or("");
        println!("{:<36}  {:<16}  {:<10}  {}", id, jtype, status, title);
    }
    Ok(())
}

// ── Get ───────────────────────────────────────────────────────────────────────

pub fn cmd_science_job(id: &str) -> Result<()> {
    let api = ApiClient::new();
    let job: Value = api.get(&format!("/api/science/jobs/{}", id))?;

    let field = |k: &str| job.get(k).and_then(|v| v.as_str()).unwrap_or("—").to_owned();
    let field_u64 = |k: &str| job.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    let field_bool = |k: &str| job.get(k).and_then(|v| v.as_bool()).unwrap_or(false);

    println!("{}", "── Scientific Job ──────────────────────────────────".bold());
    println!("{} {}", "Job ID:".bold(), field("job_id"));
    println!("{} {}", "Title:".bold(), field("title"));
    println!("{} {}", "Type:".bold(), field("job_type"));
    println!("{} {}", "Status:".bold(), field("status"));
    println!("{} {}", "Requester:".bold(), field("requester"));
    println!("{} {}", "Open Source:".bold(), field_bool("open_source"));
    println!("{} {} dreams", "Fee Paid:".bold(), field_u64("fee_paid"));
    if let Some(model) = job.get("model").and_then(|v| v.as_str()) {
        println!("{} {}", "Model:".bold(), model);
    }
    if let Some(nodes) = job.get("contributing_nodes").and_then(|v| v.as_array()) {
        if !nodes.is_empty() {
            let list: Vec<&str> = nodes.iter().filter_map(|v| v.as_str()).collect();
            println!("{} {}", "Contributing Nodes:".bold(), list.join(", "));
        }
    }
    if let Some(rh) = job.get("result_hash").and_then(|v| v.as_str()) {
        println!("{} {}", "Result Hash:".bold(), rh);
    }
    if let Some(cid) = job.get("result_blob_cid").and_then(|v| v.as_str()) {
        println!("{} {}", "Result Blob CID:".bold(), cid);
    }
    if field_bool("on_chain") {
        println!("{}", "On-chain: yes (inscribed as ScientificResult)".green());
    }
    Ok(())
}

// ── Start ─────────────────────────────────────────────────────────────────────

pub fn cmd_science_start(job_id: &str, shard_group: Option<&str>) -> Result<()> {
    let api = ApiClient::new();
    let body = json!({ "shard_group_id": shard_group });
    let res: Value = api.post(&format!("/api/science/jobs/{}/start", job_id), &body)?;
    let status = res.get("status").and_then(|v| v.as_str()).unwrap_or("?");
    println!("{} {} → {}", "Job".bold(), job_id, status.green().bold());
    Ok(())
}

// ── Complete ──────────────────────────────────────────────────────────────────

pub fn cmd_science_complete(
    job_id: &str,
    result_hash: &str,
    result: &str,
    nodes: &[String],
    epoch: u64,
) -> Result<()> {
    let api = ApiClient::new();
    let result_bytes = read_input(result);

    let body = json!({
        "result_hash": result_hash,
        "result_b64": base64_encode(&result_bytes),
        "contributing_nodes": nodes,
        "epoch": epoch,
    });

    let res: Value = api.post(&format!("/api/science/jobs/{}/complete", job_id), &body)?;
    let status = res.get("status").and_then(|v| v.as_str()).unwrap_or("?");
    let on_chain = res.get("on_chain").and_then(|v| v.as_bool()).unwrap_or(false);

    println!("{} {} → {}", "Job".bold(), job_id, status.green().bold());
    if on_chain {
        println!("{}", "Inscribed on-chain as ScientificResult.".green());
    }
    if let Some(cid) = res.get("result_blob_cid").and_then(|v| v.as_str()) {
        println!("{} {}", "Blob CID:".bold(), cid);
    }
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn base64_encode(data: &[u8]) -> String {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    STANDARD.encode(data)
}

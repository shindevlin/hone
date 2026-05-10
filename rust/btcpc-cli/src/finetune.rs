use anyhow::Result;
use btcpc_sdk::KeyPair;
use colored::Colorize;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;
use crate::helpers::{next_nonce, node_chain_id, print_resp, resolve_key_file, sign_entry};

// ── Fine-tune ─────────────────────────────────────────────────────────────────

pub fn cmd_finetune_post(
    account: &str, base_model: &str, dataset_cid: &str,
    lora_rank: u32, max_fee: u64, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "FINE_TUNE_JOB_POST", json!({
        "account": account, "base_model": base_model, "dataset_cid": dataset_cid,
        "lora_rank": lora_rank, "max_fee": max_fee, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/finetune/job", &json!({
        "account": account, "base_model": base_model, "dataset_cid": dataset_cid,
        "lora_rank": lora_rank, "max_fee": max_fee, "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Fine-tune job posted.".green().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_finetune_complete(
    worker: &str, job_id: &str, adapter_cid: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, worker)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "FINE_TUNE_JOB_COMPLETE", json!({
        "worker": worker, "job_id": job_id, "adapter_cid": adapter_cid, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/finetune/complete", &json!({
        "worker": worker, "job_id": job_id, "adapter_cid": adapter_cid,
        "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Fine-tune job completed.".green().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_finetune_get(job_id: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/finetune/job/{}", job_id))?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

pub fn cmd_finetune_jobs() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/finetune/jobs/open")?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

// ── Computer-use ──────────────────────────────────────────────────────────────

pub fn cmd_cu_post(
    account: &str, task_json: &str, max_fee: u64, key_file: Option<&Path>,
) -> Result<()> {
    let task: Value = serde_json::from_str(task_json)
        .map_err(|e| anyhow::anyhow!("invalid task JSON: {}", e))?;
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "COMPUTER_USE_JOB_POST", json!({
        "account": account, "max_fee": max_fee, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/computer-use/job", &json!({
        "account": account, "task_json": task, "max_fee": max_fee,
        "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Computer-use job posted.".green().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_cu_get(job_id: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/computer-use/job/{}", job_id))?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

pub fn cmd_cu_jobs() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/computer-use/jobs/open")?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

// ── Snapshot ──────────────────────────────────────────────────────────────────

pub fn cmd_snap_save(account: &str, slug: &str, cid: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "SNAPSHOT_SAVE", json!({
        "account": account, "slug": slug, "cid": cid, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/snapshot/save", &json!({
        "account": account, "slug": slug, "cid": cid, "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Snapshot saved.".green().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_snap_get(account: &str, slug: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/snapshot/{}/{}", account, slug))?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

pub fn cmd_snap_list(account: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/snapshot/list/{}", account))?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

// ── Amber Pill ────────────────────────────────────────────────────────────────

pub fn cmd_amber_pill_mint(account: &str, device_fingerprint: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AMBER_PILL_MINT", json!({
        "account": account, "device_fingerprint": device_fingerprint, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/amber-pill/mint", &json!({
        "account": account, "device_fingerprint": device_fingerprint,
        "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Amber Pill minted.".yellow().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_amber_pill_get(account: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/amber-pill/{}", account))?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

// ── Fee / Mempool ─────────────────────────────────────────────────────────────

pub fn cmd_fee_estimate() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/chain/fee-estimate")?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_mempool_status() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/mempool/status")?;
    print_resp(&resp);
    Ok(())
}

// ── Peer Commerce ─────────────────────────────────────────────────────────────

pub fn cmd_peer_register(
    account: &str, product_cid: &str, price: u64, description: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "PEER_COMMERCE_REGISTER", json!({
        "account": account, "product_cid": product_cid, "price": price, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/peer-commerce/register", &json!({
        "account": account, "product_cid": product_cid, "price": price,
        "description": description, "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Product registered.".green().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_peer_list() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/peer-commerce/list")?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

// ── Gateway Resolver ──────────────────────────────────────────────────────────

pub fn cmd_gateway_resolve(shortcode: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/gateway/resolve/{}", shortcode))?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

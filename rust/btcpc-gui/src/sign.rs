use anyhow::{anyhow, Result};
use btcpc_sdk::KeyPair;
use serde_json::{json, Value};
use std::path::Path;

pub fn get_nonce(base: &str, account: &str) -> Result<u64> {
    let v: Value = crate::api::get_json(base, &format!("/api/account/{}", account))?;
    v.get("nonce").and_then(|n| n.as_u64())
        .ok_or_else(|| anyhow!("account '{}' not found or has no nonce", account))
        .map(|n| n + 1)
}

pub fn get_chain_id(base: &str) -> Result<String> {
    let v: Value = crate::api::get_json(base, "/api/node/info")?;
    v.get("chain_id").and_then(|s| s.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("node/info missing chain_id"))
}

pub fn submit_transfer(base: &str, key_file: &Path, from: &str, to: &str, amount_dreams: u64) -> Result<String> {
    if !key_file.exists() {
        return Err(anyhow!("key file not found: {}", key_file.display()));
    }
    let keypair = KeyPair::from_file(key_file)?;
    let nonce = get_nonce(base, from)?;
    let chain_id = get_chain_id(base)?;
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": "TRANSFER",
        "from": from,
        "to": to,
        "amount": amount_dreams,
        "token": "BTCPC",
        "nonce": nonce,
    }))?;
    let sig = keypair.sign_entry_json(&msg);
    let body = json!({
        "from": from,
        "to": to,
        "amount": amount_dreams,
        "token": "BTCPC",
        "signed_by": from,
        "nonce": nonce,
        "signature": sig,
    });
    let resp: Value = crate::api::post_json(base, "/api/transfer", &body)?;
    if resp.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false) {
        let hash = resp.get("hash").and_then(|v| v.as_str()).unwrap_or("ok");
        Ok(format!("Transfer accepted. hash: {}", &hash[..hash.len().min(16)]))
    } else {
        Err(anyhow!("{}", resp.get("error").and_then(|v| v.as_str()).unwrap_or("rejected")))
    }
}

pub fn submit_stake(base: &str, key_file: &Path, account: &str, amount_dreams: u64, add: bool) -> Result<String> {
    if !key_file.exists() {
        return Err(anyhow!("key file not found: {}", key_file.display()));
    }
    let keypair = KeyPair::from_file(key_file)?;
    let nonce = get_nonce(base, account)?;
    let chain_id = get_chain_id(base)?;
    let kind = if add { "STAKE" } else { "UNSTAKE" };
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": kind,
        "account": account,
        "amount": amount_dreams,
        "nonce": nonce,
    }))?;
    let sig = keypair.sign_entry_json(&msg);
    let path = if add { "/api/stake" } else { "/api/unstake" };
    let body = json!({
        "account": account,
        "amount": amount_dreams,
        "signed_by": account,
        "nonce": nonce,
        "signature": sig,
    });
    let resp: Value = crate::api::post_json(base, path, &body)?;
    if resp.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false) {
        Ok(format!("{} accepted.", if add { "Stake" } else { "Unstake" }))
    } else {
        Err(anyhow!("{}", resp.get("error").and_then(|v| v.as_str()).unwrap_or("rejected")))
    }
}

pub fn submit_post_job(
    base: &str,
    key_file: &Path,
    account: &str,
    model: &str,
    input: &str,
    max_fee: u64,
    deadline: u64,
) -> Result<String> {
    if !key_file.exists() {
        return Err(anyhow!("key file not found: {}", key_file.display()));
    }
    let keypair = KeyPair::from_file(key_file)?;
    let nonce = get_nonce(base, account)?;
    let chain_id = get_chain_id(base)?;
    let input_hash = {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(input.as_bytes());
        hex::encode(h.finalize())
    };
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
        "mode": "solo",
        "input_hash": input_hash,
        "max_fee": max_fee,
        "min_reputation": 0,
        "bid_window_epochs": 2,
        "deadline_epoch": deadline,
        "nonce": nonce,
        "signature": sig,
    });
    let resp: Value = crate::api::post_json(base, "/api/task/post", &body)?;
    if resp.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false) {
        let job_id = resp.get("job_id").and_then(|v| v.as_str()).unwrap_or("?");
        Ok(format!("Job posted. id: {}", &job_id[..job_id.len().min(16)]))
    } else {
        Err(anyhow!("{}", resp.get("error").and_then(|v| v.as_str()).unwrap_or("rejected")))
    }
}

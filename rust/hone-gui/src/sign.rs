use anyhow::{anyhow, Context, Result};
use hone_sdk::KeyPair;
use serde_json::{json, Value};
use std::path::Path;

pub fn load_keypair(path: &Path, role: &str) -> Result<KeyPair> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read key file {}", path.display()))?;
    let v: Value = serde_json::from_str(&raw)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;

    // wallet.key format: role-specific fields (btcpc_* accepts pre-rebrand files)
    let wallet_fields: &[&str] = match role {
        "owner"   => &["hone_owner_private_key", "btcpc_owner_private_key"],
        "posting" => &["hone_private_key", "hone_posting_private_key", "btcpc_private_key"],
        _         => &["hone_active_private_key", "btcpc_active_private_key"],
    };
    for field in wallet_fields {
        if let Some(hex) = v.get(*field).and_then(|h| h.as_str()).filter(|h| !h.is_empty()) {
            return KeyPair::from_hex(hex)
                .with_context(|| format!("bad {} in {}", field, path.display()));
        }
    }
    // key.json legacy format
    if let Some(hex) = v.get("private_key_hex").and_then(|h| h.as_str()).filter(|h| !h.is_empty()) {
        return KeyPair::from_hex(hex)
            .with_context(|| format!("bad private_key_hex in {}", path.display()));
    }
    Err(anyhow!(
        "{}: no {} key found in key file",
        path.display(), role
    ))
}

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

pub fn get_epoch(base: &str) -> u64 {
    crate::api::get_json(base, "/api/node/info").ok()
        .and_then(|v| v.get("epoch").and_then(|e| e.as_u64()))
        .unwrap_or(0)
}

pub fn submit_transfer(base: &str, key_file: &Path, from: &str, to: &str, amount_hunits: u64) -> Result<String> {
    if !key_file.exists() {
        return Err(anyhow!("key file not found: {}", key_file.display()));
    }
    let keypair = load_keypair(key_file, "active")?;
    let nonce = get_nonce(base, from)?;
    let chain_id = get_chain_id(base)?;
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": "TRANSFER",
        "from": from,
        "to": to,
        "amount": amount_hunits,
        "token": "HONE",
        "nonce": nonce,
    }))?;
    let sig = keypair.sign_entry_json(&msg);
    let body = json!({
        "from": from,
        "to": to,
        "amount": amount_hunits,
        "token": "HONE",
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

pub fn submit_stake(base: &str, key_file: &Path, account: &str, amount_hunits: u64, add: bool) -> Result<String> {
    if !key_file.exists() {
        return Err(anyhow!("key file not found: {}", key_file.display()));
    }
    let keypair = load_keypair(key_file, "active")?;
    let nonce = get_nonce(base, account)?;
    let chain_id = get_chain_id(base)?;
    let kind = if add { "STAKE" } else { "UNSTAKE" };
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": kind,
        "account": account,
        "amount": amount_hunits,
        "nonce": nonce,
    }))?;
    let sig = keypair.sign_entry_json(&msg);
    let path = if add { "/api/stake" } else { "/api/unstake" };
    let body = json!({
        "account": account,
        "amount": amount_hunits,
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

pub fn submit_role_stake(
    base: &str,
    key_file: &Path,
    staker: &str,
    node: &str,
    role: &str,
    amount_hunits: u64,
    add: bool,
) -> Result<String> {
    if !key_file.exists() {
        return Err(anyhow!("key file not found: {}", key_file.display()));
    }
    let keypair = load_keypair(key_file, "active")?;
    let nonce    = get_nonce(base, staker)?;
    let chain_id = get_chain_id(base)?;
    let epoch    = get_epoch(base);
    let (kind, path) = if add {
        ("NODE_ROLE_STAKE", "/api/node/role/stake")
    } else {
        ("NODE_ROLE_UNSTAKE", "/api/node/role/unstake")
    };
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": kind,
        "node": node,
        "role": role,
        "staker": staker,
        "amount": amount_hunits,
        "nonce": nonce,
    }))?;
    let sig = keypair.sign_entry_json(&msg);
    let body = json!({
        "node": node,
        "role": role,
        "staker": staker,
        "amount": amount_hunits,
        "epoch": epoch,
        "signed_by": staker,
        "nonce": nonce,
        "signature": sig,
    });
    let resp: Value = crate::api::post_json(base, path, &body)?;
    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if ok {
        let hash = resp.get("hash").and_then(|v| v.as_str()).unwrap_or("ok");
        Ok(format!("{} accepted. hash: {}", if add { "Stake" } else { "Unstake" }, &hash[..hash.len().min(16)]))
    } else {
        Err(anyhow!("{}", resp.get("error").and_then(|v| v.as_str()).unwrap_or("rejected")))
    }
}

pub fn submit_role_opt_in(
    base: &str,
    key_file: &Path,
    node: &str,
    role: &str,
    backer_share_bps: u16,
    max_backers: u8,
) -> Result<String> {
    if !key_file.exists() {
        return Err(anyhow!("key file not found: {}", key_file.display()));
    }
    let keypair = load_keypair(key_file, "posting")?;
    let nonce = get_nonce(base, node)?;
    let chain_id = get_chain_id(base)?;
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": "NODE_ROLE_OPT_IN",
        "node": node,
        "role": role,
        "backer_share_bps": backer_share_bps,
        "max_backers": max_backers,
        "nonce": nonce,
    }))?;
    let sig = keypair.sign_entry_json(&msg);
    let body = json!({
        "node": node,
        "role": role,
        "backer_share_bps": backer_share_bps,
        "max_backers": max_backers,
        "signed_by": node,
        "nonce": nonce,
        "signature": sig,
    });
    let resp: Value = crate::api::post_json(base, "/api/node/role/opt-in", &body)?;
    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if ok {
        Ok(format!("Opted in: {} sharing {}% of {} rewards with up to {} backers",
            node, backer_share_bps / 100, role, max_backers))
    } else {
        Err(anyhow!("{}", resp.get("error").and_then(|v| v.as_str()).unwrap_or("rejected")))
    }
}

pub fn submit_role_opt_out(
    base: &str,
    key_file: &Path,
    node: &str,
    role: &str,
) -> Result<String> {
    if !key_file.exists() {
        return Err(anyhow!("key file not found: {}", key_file.display()));
    }
    let keypair = load_keypair(key_file, "posting")?;
    let nonce = get_nonce(base, node)?;
    let chain_id = get_chain_id(base)?;
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": "NODE_ROLE_OPT_OUT",
        "node": node,
        "role": role,
        "nonce": nonce,
    }))?;
    let sig = keypair.sign_entry_json(&msg);
    let body = json!({
        "node": node,
        "role": role,
        "signed_by": node,
        "nonce": nonce,
        "signature": sig,
    });
    let resp: Value = crate::api::post_json(base, "/api/node/role/opt-out", &body)?;
    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if ok {
        Ok(format!("Opted out of {} role sharing for '{}'", role, node))
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
    let keypair = load_keypair(key_file, "active")?;
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

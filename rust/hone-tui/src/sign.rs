use anyhow::{anyhow, Context, Result};
use hone_sdk::KeyPair;
use serde_json::{json, Value};
use std::path::Path;

// ── Key loading ───────────────────────────────────────────────────────────────

/// Load a keypair from a key file. Understands two formats:
///
/// 1. `wallet.key` — full WalletKeys bundle produced by the node.
///    Contains `hone_active_private_key`, `hone_private_key` (posting), etc.
///    `role` selects which key to extract: "active", "posting", "owner".
///
/// 2. `key.json` — simple standalone key: `{ "private_key_hex": "..." }`.
///    The `role` parameter is ignored; the single key is used.
pub fn load_keypair(path: &Path, role: &str) -> Result<KeyPair> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read key file {}", path.display()))?;
    let v: Value = serde_json::from_str(&raw)
        .with_context(|| format!("invalid JSON in {}", path.display()))?;

    // wallet.key format — pick the private key for the requested role
    // btcpc_* fallbacks accept pre-rebrand wallet.key files.
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

    // simple key.json format
    if let Some(hex) = v.get("private_key_hex").and_then(|h| h.as_str()).filter(|h| !h.is_empty()) {
        return KeyPair::from_hex(hex)
            .with_context(|| format!("bad private_key_hex in {}", path.display()));
    }

    Err(anyhow!(
        "{}: unrecognised key file — expected wallet.key (hone_active_private_key) or key.json (private_key_hex)",
        path.display()
    ))
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub fn get_nonce(base: &str, account: &str) -> Result<u64> {
    let v: Value = crate::api::get_json(base, &format!("/api/account/{}", account))?;
    v.get("nonce")
        .and_then(|n| n.as_u64())
        .ok_or_else(|| anyhow!("account '{}' not found or has no nonce", account))
        .map(|n| n + 1)
}

pub fn get_chain_id(base: &str) -> Result<String> {
    let v: Value = crate::api::get_json(base, "/api/node/info")?;
    v.get("chain_id")
        .and_then(|s| s.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("node/info missing chain_id"))
}

pub fn get_epoch(base: &str) -> u64 {
    crate::api::get_json(base, "/api/node/info").ok()
        .and_then(|v| v.get("epoch").and_then(|e| e.as_u64()))
        .unwrap_or(0)
}

// ── Transfer ──────────────────────────────────────────────────────────────────

pub fn submit_transfer(
    base: &str,
    key_file: &Path,
    from: &str,
    to: &str,
    amount_hunits: u64,
    memo: &str,
) -> Result<String> {
    let keypair = load_keypair(key_file, "active")?;
    let nonce = get_nonce(base, from)?;
    let chain_id = get_chain_id(base)?;

    // Canonical signing message — must match tx::canonical_signing_message
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

    let memo_val = if memo.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::String(memo.to_owned())
    };

    let body = json!({
        "from": from,
        "to": to,
        "amount": amount_hunits,
        "token": "HONE",
        "memo": memo_val,
        "signed_by": from,
        "nonce": nonce,
        "signature": sig,
    });

    let resp: Value = crate::api::post_json(base, "/api/transfer", &body)?;
    if resp.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false) {
        let hash = resp.get("hash").and_then(|v| v.as_str()).unwrap_or("ok");
        Ok(format!("Transfer accepted. hash: {}", &hash[..hash.len().min(16)]))
    } else {
        let err = resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("rejected");
        Err(anyhow!("{}", err))
    }
}

// ── Stake / Unstake ───────────────────────────────────────────────────────────

pub fn submit_stake(
    base: &str,
    key_file: &Path,
    account: &str,
    amount_hunits: u64,
    add: bool,
) -> Result<String> {
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
        let err = resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("rejected");
        Err(anyhow!("{}", err))
    }
}

// ── Role stake / unstake ──────────────────────────────────────────────────────

pub fn submit_role_stake(
    base: &str,
    key_file: &Path,
    staker: &str,
    node: &str,
    role: &str,
    amount_hunits: u64,
    add: bool,
) -> Result<String> {
    let keypair  = load_keypair(key_file, "active")?;
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
    let ok = resp.get("ok").and_then(|v| v.as_bool())
        .or_else(|| resp.get("accepted").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    if ok {
        Ok(format!("{} accepted.", if add { "Stake" } else { "Unstake" }))
    } else {
        Err(anyhow!("{}", resp.get("error").and_then(|v| v.as_str()).unwrap_or("rejected")))
    }
}

// ── Inference job post ────────────────────────────────────────────────────────

pub fn submit_post_job(
    base: &str,
    key_file: &Path,
    account: &str,
    model: &str,
    input: &str,
    max_fee: u64,
    deadline: u64,
) -> Result<String> {
    let keypair = load_keypair(key_file, "active")?;
    let nonce = get_nonce(base, account)?;
    let chain_id = get_chain_id(base)?;

    let input_hash = {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(input.as_bytes());
        hex::encode(h.finalize())
    };

    // Canonical signing message — matches tx::canonical_signing_message for InferenceJobPost
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
        let job_id = resp
            .get("job_id")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        Ok(format!("Job posted. id: {}", &job_id[..job_id.len().min(16)]))
    } else {
        let err = resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("rejected");
        Err(anyhow!("{}", err))
    }
}

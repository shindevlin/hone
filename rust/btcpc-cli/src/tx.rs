use anyhow::{anyhow, Result};
use btcpc_sdk::KeyPair;
use colored::Colorize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::api::ApiClient;

fn print_result(resp: &Value) {
    // Try to display a useful confirmation message
    if let Some(obj) = resp.as_object() {
        for (k, v) in obj {
            let val = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => serde_json::to_string(v).unwrap_or_default(),
            };
            println!("{}: {}", k.bold(), val);
        }
    } else {
        println!("{}", serde_json::to_string_pretty(resp).unwrap_or_default());
    }
}

// ── Transfer ──────────────────────────────────────────────────────────────────

pub fn cmd_transfer(
    from: &str,
    to: &str,
    amount: u64,
    memo: Option<&str>,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let key_path = resolve_key_file(key_file)?;
    let keypair = KeyPair::from_file(&key_path)?;
    let nonce = next_nonce(&api, from)?;

    let sig = sign_transfer(&keypair, from, to, amount, "BTCPC", nonce);

    let body = json!({
        "from": from,
        "to": to,
        "amount": amount,
        "token": "BTCPC",
        "memo": memo,
        "signed_by": from,
        "nonce": nonce,
        "signature": sig,
    });

    let resp: Value = api.post("/api/transfer", &body)?;
    println!("{}", "Transfer submitted.".green().bold());
    print_result(&resp);
    Ok(())
}

// ── Stake Add ─────────────────────────────────────────────────────────────────

pub fn cmd_stake_add(account: &str, amount: u64, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let key_path = resolve_key_file(key_file)?;
    let keypair = KeyPair::from_file(&key_path)?;
    let nonce = next_nonce(&api, account)?;

    let sig = sign_stake(&keypair, account, amount, nonce);

    let body = json!({
        "account": account,
        "amount": amount,
        "signed_by": account,
        "nonce": nonce,
        "signature": sig,
    });

    let resp: Value = api.post("/api/stake", &body)?;
    println!("{}", "Stake submitted.".green().bold());
    print_result(&resp);
    Ok(())
}

// ── Stake Remove ──────────────────────────────────────────────────────────────

pub fn cmd_stake_remove(account: &str, amount: u64, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let key_path = resolve_key_file(key_file)?;
    let keypair = KeyPair::from_file(&key_path)?;
    let nonce = next_nonce(&api, account)?;

    let sig = sign_unstake(&keypair, account, amount, nonce);

    let body = json!({
        "account": account,
        "amount": amount,
        "signed_by": account,
        "nonce": nonce,
        "signature": sig,
    });

    let resp: Value = api.post("/api/unstake", &body)?;
    println!("{}", "Unstake submitted.".green().bold());
    print_result(&resp);
    Ok(())
}

// ── Account Create ────────────────────────────────────────────────────────────

pub fn cmd_account_create(account: &str, pubkey: Option<&str>) -> Result<()> {
    let api = ApiClient::new();

    let mut body = json!({
        "account": account,
    });

    if let Some(pk) = pubkey {
        body["public_key"] = json!(pk);
    }

    let resp: Value = api.post("/api/account/create", &body)?;
    println!("{}", "Account created.".green().bold());
    print_result(&resp);
    Ok(())
}

fn resolve_key_file(key_file: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = key_file {
        return Ok(path.to_path_buf());
    }

    if let Ok(path) = std::env::var("BTCPC_KEY_FILE") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    Err(anyhow!(
        "missing key file: pass --key-file <path> or set BTCPC_KEY_FILE"
    ))
}

fn next_nonce(api: &ApiClient, account: &str) -> Result<u64> {
    let path = format!("/api/account/{}", account);
    let account_data: Value = api.get(&path)?;
    let current = account_data
        .get("nonce")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("account '{}' has no nonce field", account))?;
    current
        .checked_add(1)
        .ok_or_else(|| anyhow!("nonce overflow for account '{}'", account))
}

/// Build and sign the canonical message that the node's check_signature verifies.
/// Field names and order must exactly match `tx::canonical_signing_message`.
fn sign_transfer(keypair: &KeyPair, from: &str, to: &str, amount: u64, token: &str, nonce: u64) -> String {
    let msg = serde_json::to_string(&serde_json::json!({
        "type": "TRANSFER",
        "from": from,
        "to": to,
        "amount": amount,
        "token": token,
        "nonce": nonce,
    })).unwrap_or_default();
    keypair.sign_entry_json(&msg)
}

fn sign_stake(keypair: &KeyPair, account: &str, amount: u64, nonce: u64) -> String {
    let msg = serde_json::to_string(&serde_json::json!({
        "type": "STAKE",
        "account": account,
        "amount": amount,
        "nonce": nonce,
    })).unwrap_or_default();
    keypair.sign_entry_json(&msg)
}

fn sign_unstake(keypair: &KeyPair, account: &str, amount: u64, nonce: u64) -> String {
    let msg = serde_json::to_string(&serde_json::json!({
        "type": "UNSTAKE",
        "account": account,
        "amount": amount,
        "nonce": nonce,
    })).unwrap_or_default();
    keypair.sign_entry_json(&msg)
}

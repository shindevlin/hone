use anyhow::{anyhow, Result};
use hone_sdk::KeyPair;
use colored::Colorize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::api::ApiClient;
use crate::session;

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
    let chain_id = node_chain_id(&api)?;

    let sig = sign_transfer(&keypair, &chain_id, from, to, amount, "HONE", nonce);

    let body = json!({
        "from": from,
        "to": to,
        "amount": amount,
        "token": "HONE",
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
    let chain_id = node_chain_id(&api)?;

    let sig = sign_stake(&keypair, &chain_id, account, amount, nonce);

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
    let chain_id = node_chain_id(&api)?;

    let sig = sign_unstake(&keypair, &chain_id, account, amount, nonce);

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

pub fn cmd_account_create(account: &str, pubkey: Option<&str>, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let key_path = resolve_key_file(key_file)?;
    let keypair = KeyPair::from_file(&key_path)?;
    let local_pubkey = keypair.public_key_hex();
    let public_key = pubkey.unwrap_or(local_pubkey.as_str());
    if public_key != local_pubkey {
        return Err(anyhow!(
            "cannot create a signed account for pubkey {} with local key {}; omit --pubkey or use the matching key file",
            public_key,
            local_pubkey
        ));
    }
    let chain_id = node_chain_id(&api)?;
    let keys = serde_json::json!({
        "owner": public_key,
        "active": public_key,
        "posting": public_key,
    });
    let sig = sign_account_create(&keypair, &chain_id, account, public_key)?;

    let body = json!({
        "account": account,
        "keys": keys,
        "signature": sig,
    });

    let resp: Value = api.post("/api/account/create", &body)?;
    println!("{}", "Account created.".green().bold());
    print_result(&resp);
    Ok(())
}

fn sign_account_create(keypair: &KeyPair, chain_id: &str, account: &str, pubkey: &str) -> Result<String> {
    let keys = serde_json::json!({
        "active": pubkey,
        "owner": pubkey,
        "posting": pubkey,
    });
    let msg = serde_json::to_string(&serde_json::json!({
        "chain_id": chain_id,
        "type": "ACCOUNT_CREATE",
        "account": account,
        "keys": keys,
        "chain_proofs": [],
        "funded_by": null,
    }))?;
    Ok(keypair.sign_entry_json(&msg))
}

fn resolve_key_file(key_file: Option<&Path>) -> Result<PathBuf> {
    session::resolve_key_file(key_file, session::load().as_ref())
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

fn node_chain_id(api: &ApiClient) -> Result<String> {
    let info: Value = api.get("/api/node/info")?;
    info.get("chain_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("node info missing chain_id"))
}

/// Build and sign the canonical message that the node's check_signature verifies.
/// Field order is BTreeMap alphabetical — must exactly match `tx::canonical_signing_message`.
fn sign_transfer(keypair: &KeyPair, chain_id: &str, from: &str, to: &str, amount: u64, token: &str, nonce: u64) -> String {
    let msg = serde_json::to_string(&serde_json::json!({
        "chain_id": chain_id,
        "type": "TRANSFER",
        "from": from,
        "to": to,
        "amount": amount,
        "token": token,
        "nonce": nonce,
    })).unwrap_or_default();
    keypair.sign_entry_json(&msg)
}

fn sign_stake(keypair: &KeyPair, chain_id: &str, account: &str, amount: u64, nonce: u64) -> String {
    let msg = serde_json::to_string(&serde_json::json!({
        "chain_id": chain_id,
        "type": "STAKE",
        "account": account,
        "amount": amount,
        "nonce": nonce,
    })).unwrap_or_default();
    keypair.sign_entry_json(&msg)
}

fn sign_unstake(keypair: &KeyPair, chain_id: &str, account: &str, amount: u64, nonce: u64) -> String {
    let msg = serde_json::to_string(&serde_json::json!({
        "chain_id": chain_id,
        "type": "UNSTAKE",
        "account": account,
        "amount": amount,
        "nonce": nonce,
    })).unwrap_or_default();
    keypair.sign_entry_json(&msg)
}

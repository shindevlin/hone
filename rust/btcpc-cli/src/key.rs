use anyhow::{anyhow, Result};
use btcpc_sdk::KeyPair;
use colored::Colorize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::api::ApiClient;

fn node_chain_id(api: &ApiClient) -> anyhow::Result<String> {
    let info: serde_json::Value = api.get("/api/node/info")?;
    info.get("chain_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("node info missing chain_id"))
}

pub fn cmd_key_generate(output: Option<&Path>) -> Result<()> {
    let path = resolve_key_path(output)?;
    let keypair = KeyPair::generate();
    keypair.save_to_file(&path)?;

    println!("{}", "Key generated.".green().bold());
    println!("{} {}", "File:".bold(), path.display());
    println!("{} {}", "Public key:".bold(), keypair.public_key_hex());
    Ok(())
}

pub fn cmd_key_show(key_file: Option<&Path>) -> Result<()> {
    let path = resolve_key_path(key_file)?;
    let keypair = KeyPair::from_file(&path)?;

    println!("{} {}", "File:".bold(), path.display());
    println!("{} {}", "Public key:".bold(), keypair.public_key_hex());
    Ok(())
}

pub fn cmd_key_register(account: &str, role: Option<&str>, key_file: Option<&Path>) -> Result<()> {
    let path = resolve_key_path(key_file)?;
    let keypair = KeyPair::from_file(&path)?;
    let pubkey = keypair.public_key_hex();
    let role = role.unwrap_or("posting");

    let api = ApiClient::new();
    let chain_id = node_chain_id(&api)?;

    // Canonical message — field order is alphabetical (serde_json BTreeMap).
    // Must match tx.rs::canonical_signing_message for AccountUpdateKey.
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": "ACCOUNT_UPDATE_KEY",
        "account": account,
        "role": role,
        "new_public_key": pubkey,
    }))?;
    let sig = keypair.sign_entry_json(&msg);

    let body = json!({
        "account": account,
        "role": role,
        "new_public_key": pubkey,
        "signed_by": account,
        "signature": sig,
    });

    let resp: Value = api.post("/api/account/update-key", &body)?;
    if resp.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false) {
        println!("{}", "Key registered on-chain.".green().bold());
        if let Some(h) = resp.get("hash").and_then(|v| v.as_str()) {
            println!("hash: {}", h);
        }
    } else {
        let err = resp.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
        return Err(anyhow!("{}", err));
    }
    Ok(())
}

fn resolve_key_path(input: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = input {
        return Ok(path.to_path_buf());
    }
    default_key_path()
}

fn default_key_path() -> Result<PathBuf> {
    let home = std::env::var("HOME")
        .map_err(|_| anyhow!("HOME is not set; pass --key-file explicitly"))?;
    Ok(PathBuf::from(home).join(".btcpc").join("key.json"))
}


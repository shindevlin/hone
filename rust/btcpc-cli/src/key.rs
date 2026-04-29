use anyhow::{anyhow, Result};
use btcpc_sdk::KeyPair;
use colored::Colorize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::api::ApiClient;

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

pub fn cmd_key_register(account: &str, key_file: Option<&Path>) -> Result<()> {
    let path = resolve_key_path(key_file)?;
    let keypair = KeyPair::from_file(&path)?;
    let pubkey = keypair.public_key_hex();

    let api = ApiClient::new();

    // Sign the canonical AccountUpdateKey message.
    // If this is a first-time registration (no key on chain), the node accepts
    // the entry without a signature.  If there is already a key, this signature
    // proves ownership of the existing key before rotation.
    let msg = serde_json::to_string(&json!({
        "type": "ACCOUNT_UPDATE_KEY",
        "account": account,
        "new_public_key": pubkey,
    }))?;
    let sig = keypair.sign_entry_json(&msg);

    let body = json!({
        "account": account,
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


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

    let api = ApiClient::new();
    let body = json!({
        "account": account,
        "public_key": keypair.public_key_hex(),
    });

    let resp: Value = api.post("/api/account/create", &body)?;
    println!("{}", "Key registered on-chain.".green().bold());
    print_result(&resp);
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

fn print_result(resp: &Value) {
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

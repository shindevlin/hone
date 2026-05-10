use anyhow::{anyhow, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::api::ApiClient;
use crate::session;

pub fn resolve_key_file(key_file: Option<&Path>) -> Result<PathBuf> {
    session::resolve_key_file(key_file, session::load().as_ref())
}

pub fn next_nonce(api: &ApiClient, account: &str) -> Result<u64> {
    let path = format!("/api/account/{}", account);
    let data: Value = api.get(&path)?;
    let current = data
        .get("nonce")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("account '{}' has no nonce field", account))?;
    current.checked_add(1).ok_or_else(|| anyhow!("nonce overflow"))
}

pub fn node_chain_id(api: &ApiClient) -> Result<String> {
    let info: Value = api.get("/api/node/info")?;
    info.get("chain_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("node info missing chain_id"))
}

pub fn sign_entry(
    keypair: &btcpc_sdk::KeyPair,
    chain_id: &str,
    entry_type: &str,
    mut fields: serde_json::Value,
) -> String {
    fields["chain_id"] = chain_id.into();
    fields["type"] = entry_type.into();
    let msg = serde_json::to_string(&fields).unwrap_or_default();
    keypair.sign_entry_json(&msg)
}

pub fn print_resp(v: &Value) {
    if let Some(obj) = v.as_object() {
        use colored::Colorize;
        for (k, val) in obj {
            let s = match val {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => serde_json::to_string(val).unwrap_or_default(),
            };
            println!("{}: {}", k.bold(), s);
        }
    } else {
        println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
    }
}

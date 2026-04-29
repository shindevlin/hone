use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use btcpc_sdk::KeyPair;
use colored::Colorize;
use serde_json::{json, Value};
use std::{fs, path::{Path, PathBuf}};

use crate::api::ApiClient;

fn resolve_key(key_file: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = key_file {
        return Ok(p.to_path_buf());
    }
    if let Ok(p) = std::env::var("BTCPC_KEY_FILE") {
        let t = p.trim();
        if !t.is_empty() {
            return Ok(PathBuf::from(t));
        }
    }
    Err(anyhow!("missing key file: pass --key-file <path> or set BTCPC_KEY_FILE"))
}

fn next_nonce(api: &ApiClient, account: &str) -> Result<u64> {
    let data: Value = api.get(&format!("/api/account/{}", account))?;
    let current = data.get("nonce").and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("account '{}' has no nonce field", account))?;
    current.checked_add(1).ok_or_else(|| anyhow!("nonce overflow"))
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

// ── Deploy ────────────────────────────────────────────────────────────────────

pub fn cmd_contract_deploy(
    deployer: &str,
    wasm_path: &str,
    init_method: Option<&str>,
    init_args: Option<&str>,
) -> Result<()> {
    let wasm_bytes =
        fs::read(wasm_path).with_context(|| format!("failed to read wasm file: {}", wasm_path))?;
    let wasm_b64 = B64.encode(&wasm_bytes);

    let api = ApiClient::new();
    let key_path = resolve_key(None)?;
    let keypair = KeyPair::from_file(&key_path)?;
    let nonce = next_nonce(&api, deployer)?;

    // Sign canonical deploy message.
    let epoch: u64 = api.get::<Value>("/api/latest")?
        .get("current_epoch").and_then(|v| v.as_u64()).unwrap_or(0);
    let msg = serde_json::to_string(&json!({
        "type": "CONTRACT_DEPLOY",
        "deployer": deployer,
        "nonce": nonce,
        "epoch": epoch,
    }))?;
    let sig = keypair.sign_entry_json(&msg);

    let mut body = json!({
        "deployer": deployer,
        "wasm_b64": wasm_b64,
        "gas": 300_000_000_000u64,
        "nonce": nonce,
        "signature": sig,
    });

    if let Some(m) = init_method {
        body["init_method"] = json!(m);
    }
    if let Some(a) = init_args {
        match serde_json::from_str::<Value>(a) {
            Ok(v) => body["init_args"] = v,
            Err(_) => body["init_args"] = json!(a),
        }
    }

    let resp: Value = api.post("/api/contract/deploy", &body)?;
    if resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        println!("{}", "Contract deployed.".green().bold());
    } else {
        let err = resp.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
        return Err(anyhow!("{}", err));
    }
    print_result(&resp);
    Ok(())
}

// ── Call ──────────────────────────────────────────────────────────────────────

pub fn cmd_contract_call(
    contract_id: &str,
    method: &str,
    signer: &str,
    args: Option<&str>,
    deposit: Option<u64>,
) -> Result<()> {
    let api = ApiClient::new();
    let key_path = resolve_key(None)?;
    let keypair = KeyPair::from_file(&key_path)?;
    let nonce = next_nonce(&api, signer)?;

    let epoch: u64 = api.get::<Value>("/api/latest")?
        .get("current_epoch").and_then(|v| v.as_u64()).unwrap_or(0);
    let msg = serde_json::to_string(&json!({
        "type": "CONTRACT_CALL",
        "signer": signer,
        "contract_id": contract_id,
        "method": method,
        "nonce": nonce,
        "epoch": epoch,
    }))?;
    let sig = keypair.sign_entry_json(&msg);

    let mut body = json!({
        "contract_id": contract_id,
        "method": method,
        "signer": signer,
        "gas": 300_000_000_000u64,
        "nonce": nonce,
        "signature": sig,
    });

    if let Some(a) = args {
        match serde_json::from_str::<Value>(a) {
            Ok(v) => body["args"] = v,
            Err(_) => body["args"] = json!(a),
        }
    }
    if let Some(d) = deposit {
        body["deposit"] = json!(d);
    }

    let resp: Value = api.post("/api/contract/call", &body)?;
    if !resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
        let err = resp.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
        return Err(anyhow!("{}", err));
    }
    println!("{}", "Contract call result:".bold());
    print_result(&resp);
    Ok(())
}

// ── View ──────────────────────────────────────────────────────────────────────

pub fn cmd_contract_view(
    contract_id: &str,
    method: &str,
    args: Option<&str>,
) -> Result<()> {
    let api = ApiClient::new();
    let mut body = json!({
        "contract_id": contract_id,
        "method": method,
        "gas": 300_000_000_000u64,
    });

    if let Some(a) = args {
        match serde_json::from_str::<Value>(a) {
            Ok(v) => body["args"] = v,
            Err(_) => body["args"] = json!(a),
        }
    }

    let resp: Value = api.post("/api/contract/view", &body)?;
    println!("{}", "View result:".bold());
    print_result(&resp);
    Ok(())
}

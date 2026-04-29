use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use colored::Colorize;
use serde_json::{json, Value};
use std::fs;

use crate::api::ApiClient;

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
    let mut body = json!({
        "deployer": deployer,
        "wasm_b64": wasm_b64,
        "gas": 10_000_000,
    });

    if let Some(m) = init_method {
        body["init_method"] = json!(m);
    }
    if let Some(a) = init_args {
        // parse as JSON if possible, otherwise store as string
        match serde_json::from_str::<Value>(a) {
            Ok(v) => body["init_args"] = v,
            Err(_) => body["init_args"] = json!(a),
        }
    }

    let resp: Value = api.post("/api/contract/deploy", &body)?;
    println!("{}", "Contract deployed.".green().bold());
    print_result(&resp);
    Ok(())
}

// ── Call ──────────────────────────────────────────────────────────────────────

pub fn cmd_contract_call(
    contract_id: &str,
    method: &str,
    signer: &str,
    args: Option<&str>,
    deposit: Option<f64>,
) -> Result<()> {
    let api = ApiClient::new();
    let mut body = json!({
        "contract_id": contract_id,
        "method": method,
        "signer": signer,
        "gas": 10_000_000,
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
        "gas": 10_000_000,
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

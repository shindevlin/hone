use anyhow::Result;
use colored::Colorize;
use serde_json::{json, Value};

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
    amount: f64,
    memo: Option<&str>,
    sig: Option<&str>,
) -> Result<()> {
    let api = ApiClient::new();

    let mut body = json!({
        "from": from,
        "to": to,
        "amount": amount,
    });

    if let Some(m) = memo {
        body["memo"] = json!(m);
    }
    if let Some(s) = sig {
        body["signature"] = json!(s);
        body["signed_by"] = json!(from);
    }

    let resp: Value = api.post("/api/transfer", &body)?;
    println!("{}", "Transfer submitted.".green().bold());
    print_result(&resp);
    Ok(())
}

// ── Stake Add ─────────────────────────────────────────────────────────────────

pub fn cmd_stake_add(account: &str, amount: f64, sig: Option<&str>) -> Result<()> {
    let api = ApiClient::new();

    let mut body = json!({
        "account": account,
        "amount": amount,
    });

    if let Some(s) = sig {
        body["signature"] = json!(s);
        body["signed_by"] = json!(account);
    }

    let resp: Value = api.post("/api/stake", &body)?;
    println!("{}", "Stake submitted.".green().bold());
    print_result(&resp);
    Ok(())
}

// ── Stake Remove ──────────────────────────────────────────────────────────────

pub fn cmd_stake_remove(account: &str, amount: f64, sig: Option<&str>) -> Result<()> {
    let api = ApiClient::new();

    let mut body = json!({
        "account": account,
        "amount": amount,
    });

    if let Some(s) = sig {
        body["signature"] = json!(s);
        body["signed_by"] = json!(account);
    }

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

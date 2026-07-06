use anyhow::Result;
use colored::Colorize;
use serde::Deserialize;
use serde_json::Value;
use tabled::{Table, Tabled};

use crate::api::ApiClient;

/// Convert hunits (u64) to HONE string with 10 decimal places.
pub fn hunits_to_hone(hunits: u64) -> String {
    let whole = hunits / 10_000_000_000;
    let frac = hunits % 10_000_000_000;
    format!("{}.{:010} HONE", whole, frac)
}

// ── Balance ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct BalanceResp {
    balance: Option<Value>,
    hunits: Option<u64>,
    token: Option<String>,
}

pub fn cmd_balance(account: &str, token: Option<&str>) -> Result<()> {
    let api = ApiClient::new();
    let path = format!("/api/balance/{}", account);
    let resp: BalanceResp = api.get(&path)?;

    let token_name = token
        .map(|s| s.to_string())
        .or(resp.token)
        .unwrap_or_else(|| "HONE".to_string());

    if let Some(hunits) = resp.hunits {
        println!(
            "{} {}",
            "Balance:".bold(),
            hunits_to_hone(hunits).green()
        );
        println!("{} {}", "Token:".bold(), token_name);
    } else if let Some(bal) = resp.balance {
        println!(
            "{} {} {}",
            "Balance:".bold(),
            value_to_display(&bal).green(),
            token_name
        );
    } else {
        println!("{} 0.0000000000 HONE", "Balance:".bold());
    }
    Ok(())
}

// ── Stake ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct StakeResp {
    stake: Option<Value>,
    hunits: Option<u64>,
}

pub fn cmd_stake(account: &str) -> Result<()> {
    let api = ApiClient::new();
    let path = format!("/api/stake/{}", account);
    let resp: StakeResp = api.get(&path)?;

    if let Some(hunits) = resp.hunits {
        println!("{} {}", "Staked:".bold(), hunits_to_hone(hunits).yellow());
    } else if let Some(s) = resp.stake {
        println!(
            "{} {} HONE",
            "Staked:".bold(),
            value_to_display(&s).yellow()
        );
    } else {
        println!("{} 0.0000000000 HONE", "Staked:".bold());
    }
    Ok(())
}

// ── Block ─────────────────────────────────────────────────────────────────────

#[derive(Tabled)]
struct BlockRow {
    #[tabled(rename = "Field")]
    field: String,
    #[tabled(rename = "Value")]
    value: String,
}

pub fn cmd_block(epoch: Option<u64>) -> Result<()> {
    let api = ApiClient::new();
    let path = match epoch {
        Some(e) => format!("/api/block/{}", e),
        None => {
            // get latest epoch first
            let latest: Value = api.get("/api/latest")?;
            let ep = latest
                .get("current_epoch")
                .or_else(|| latest.get("epoch"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            format!("/api/block/{}", ep)
        }
    };

    let block: Value = api.get(&path)?;

    let rows: Vec<BlockRow> = block
        .as_object()
        .map(|obj| {
            obj.iter()
                .map(|(k, v)| BlockRow {
                    field: k.clone(),
                    value: value_to_display(v),
                })
                .collect()
        })
        .unwrap_or_default();

    if rows.is_empty() {
        println!("{}", serde_json::to_string_pretty(&block)?);
    } else {
        println!("{}", Table::new(rows));
    }
    Ok(())
}

// ── Epoch ─────────────────────────────────────────────────────────────────────

pub fn cmd_epoch() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/latest")?;
    let epoch = resp
        .get("current_epoch")
        .or_else(|| resp.get("epoch"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    println!("{} {}", "Current epoch:".bold(), epoch.to_string().cyan());
    Ok(())
}

// ── Account ───────────────────────────────────────────────────────────────────

pub fn cmd_account(name: &str) -> Result<()> {
    let api = ApiClient::new();
    let path = format!("/api/account/{}", name);
    let resp: Value = api.get(&path)?;

    let rows: Vec<BlockRow> = resp
        .as_object()
        .map(|obj| {
            obj.iter()
                .map(|(k, v)| BlockRow {
                    field: k.clone(),
                    value: value_to_display(v),
                })
                .collect()
        })
        .unwrap_or_default();

    if rows.is_empty() {
        println!("{}", serde_json::to_string_pretty(&resp)?);
    } else {
        println!("{}", Table::new(rows));
    }
    Ok(())
}

// ── Health ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct HealthResp {
    status: Option<String>,
}

pub fn cmd_health() -> Result<()> {
    let api = ApiClient::new();
    let resp: HealthResp = api.get("/health")?;
    let status = resp.status.as_deref().unwrap_or("unknown");
    let colored_status = if status.eq_ignore_ascii_case("ok")
        || status.eq_ignore_ascii_case("healthy")
        || status.eq_ignore_ascii_case("up")
    {
        status.green().to_string()
    } else {
        status.red().to_string()
    };
    println!("{} {}", "Node status:".bold(), colored_status);
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn value_to_display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
        _ => serde_json::to_string(v).unwrap_or_default(),
    }
}

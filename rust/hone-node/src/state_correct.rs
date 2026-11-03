//! `hone-node state-correct <data_dir> <correction.json>` — one-time operator balance
//! correction on a STOPPED node's state dir. Used once (2026-11-03) to remove the storage
//! rewards nodes paid to themselves before node entries went through consensus; see
//! docs/STATE_RECONCILIATION_2026-11-03.md. The file names an id, who authorised it, the
//! rule, and `deltas_hunits` {account: signed hunits}. The tool refuses unless the deltas
//! sum to zero (no mint, no burn) and no balance goes negative, refuses a second run of the
//! same id, applies every row in ONE atomic write, adds the recycle delta to
//! supply:recycled_cumulative, and records the file under `operator_correction:{id}`.
use std::collections::BTreeMap;
use anyhow::{bail, Context, Result};

pub fn run(dir: &str, file: &str) -> Result<()> {
    let raw = std::fs::read(file).with_context(|| format!("reading {file}"))?;
    let c: serde_json::Value = serde_json::from_slice(&raw)?;
    let id = c["id"].as_str().context("id")?;
    let deltas: BTreeMap<String, i128> = c["deltas_hunits"].as_object().context("deltas_hunits")?
        .iter().map(|(k, v)| Ok((k.clone(), v.as_i64().context("delta not an integer")? as i128)))
        .collect::<Result<_>>()?;
    let store = crate::store::Store::open(&std::path::Path::new(dir).join("state"))?;
    let rec = format!("operator_correction:{id}");
    if store.state_get(&rec).is_some() { bail!("correction '{id}' already applied"); }
    let (rows, recycle) = plan(&store, &deltas)?;
    let before = store.balance_merkle_root();
    store.write_balances_atomic(&rows)?;
    if recycle != 0 {
        let prev: i128 = store.state_get("supply:recycled_cumulative")
            .and_then(|v| String::from_utf8(v).ok()).and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        store.state_set("supply:recycled_cumulative", (prev + recycle).to_string().as_bytes())?;
    }
    store.state_set(&rec, &raw)?;
    println!("correction '{id}' applied: balance root {before} -> {}", store.balance_merkle_root());
    Ok(())
}

/// The new absolute balance rows for `deltas`, and the recycle-fund delta.
pub fn plan(store: &crate::store::Store, deltas: &BTreeMap<String, i128>) -> Result<(Vec<(String, u64)>, i128)> {
    if deltas.values().sum::<i128>() != 0 { bail!("deltas must sum to zero"); }
    let mut rows = Vec::new();
    for (acct, d) in deltas {
        let cur = store.get_balance(acct, hone_types::NATIVE_TOKEN) as i128;
        let new = cur + d;
        if new < 0 || new > u64::MAX as i128 { bail!("'{acct}' would be {new}"); }
        rows.push((format!("{}\0{}", acct, hone_types::NATIVE_TOKEN), new as u64));
    }
    Ok((rows, *deltas.get(hone_types::RECYCLE_FUND_ACCOUNT).unwrap_or(&0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn applies_once_conserves_and_refuses_bad_files() {
        let d = tempfile::tempdir().unwrap();
        {
            let s = crate::store::Store::open(&d.path().join("state")).unwrap();
            s.set_balance("natoshisakamoto", "HONE", 1_000).unwrap();
            s.set_balance(hone_types::RECYCLE_FUND_ACCOUNT, "HONE", 5).unwrap();
        }
        let f = d.path().join("c.json");
        let w = |deltas: serde_json::Value| std::fs::write(&f, serde_json::json!({"id": "t", "deltas_hunits": deltas}).to_string()).unwrap();
        w(serde_json::json!({"natoshisakamoto": -10, "__recycle_fund__": 9}));
        assert!(run(d.path().to_str().unwrap(), f.to_str().unwrap()).is_err(), "must conserve");
        w(serde_json::json!({"natoshisakamoto": -2000, "__recycle_fund__": 2000}));
        assert!(run(d.path().to_str().unwrap(), f.to_str().unwrap()).is_err(), "no negative balance");
        w(serde_json::json!({"natoshisakamoto": -400, "__recycle_fund__": 400}));
        run(d.path().to_str().unwrap(), f.to_str().unwrap()).unwrap();
        assert!(run(d.path().to_str().unwrap(), f.to_str().unwrap()).is_err(), "once only");
        let s = crate::store::Store::open(&d.path().join("state")).unwrap();
        assert_eq!(s.get_balance("natoshisakamoto", "HONE"), 600);
        assert_eq!(s.get_balance(hone_types::RECYCLE_FUND_ACCOUNT, "HONE"), 405);
        assert_eq!(s.state_get("supply:recycled_cumulative").unwrap(), b"400");
    }
}

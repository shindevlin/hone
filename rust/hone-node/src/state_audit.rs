//! `hone-node state-audit <data_dir> [account,...]` — offline read of a COPY of a node's
//! state dir (never point it at a running node's dir). Prints JSON: per (account, role)
//! the number of txhist records and the sum of their `amount`, the first/last epoch, and
//! the current balance of each listed account plus the system accounts. Used to derive
//! the reconciliation for the self-paid storage drift from the actual reward entries.
use std::collections::BTreeMap;

pub fn run(dir: &str, accounts: &[String]) -> anyhow::Result<()> {
    let store = crate::store::Store::open(&std::path::Path::new(dir).join("state"))?;
    // (account, role) -> (count, sum, first_epoch, last_epoch)
    let mut agg: BTreeMap<(String, String), (u64, u128, u64, u64)> = BTreeMap::new();
    let mut per_epoch: BTreeMap<String, Vec<(u64, u64)>> = BTreeMap::new();
    let mut all_rewards: Vec<(String, String, u64, u64)> = Vec::new();
    let want = |a: &str| accounts.is_empty() || accounts.iter().any(|x| x == a);
    store.for_each_meta_prefix("txhist:", |key, val| {
        // txhist:{account}:{epoch:016x}:{role}:{hash}
        let parts: Vec<&str> = key.splitn(5, ':').collect();
        if parts.len() < 4 || !want(parts[1]) { return; }
        let Ok(epoch) = u64::from_str_radix(parts[2], 16) else { return };
        let amount = serde_json::from_slice::<serde_json::Value>(val).ok()
            .and_then(|r| r["entry"]["amount"].as_u64()).unwrap_or(0);
        let e = agg.entry((parts[1].to_owned(), parts[3].to_owned())).or_insert((0, 0, u64::MAX, 0));
        e.0 += 1; e.1 += amount as u128; e.2 = e.2.min(epoch); e.3 = e.3.max(epoch);
        if parts[3] == "storage_reward" {
            per_epoch.entry(parts[1].to_owned()).or_default().push((epoch, amount));
        }
        if parts[3].ends_with("_reward") { all_rewards.push((parts[3].to_owned(), parts[1].to_owned(), epoch, amount)); }
    });
    // STATE_AUDIT_DUMP=<file>: one CSV line per storage reward (r,account,epoch,amount) and
    // per storage_beat row (b,node,epoch,bytes_proven) — for a per-epoch cross-node diff.
    if let Ok(path) = std::env::var("STATE_AUDIT_DUMP") {
        use std::io::Write;
        let mut f = std::io::BufWriter::new(std::fs::File::create(&path)?);
        for (a, v) in &per_epoch { for (e, x) in v { writeln!(f, "r,{a},{e},{x}")?; } }
        for (role, a, e, x) in &all_rewards { writeln!(f, "w,{role}|{a},{e},{x}")?; }
        store.for_each_meta_prefix("storage_beat:", |key, val| {
            let p: Vec<&str> = key.splitn(3, ':').collect();
            if p.len() < 3 { return; }
            let bytes = serde_json::from_slice::<serde_json::Value>(val).ok()
                .and_then(|j| j["bytes_proven"].as_u64()).unwrap_or(0);
            let _ = writeln!(f, "b,{},{},{}", p[2], p[1], bytes);
        });
    }
    // STATE_AUDIT_PREFIXES=<file>: per meta-key prefix (text before the first ':' or the
    // whole key), the row count and a sha256 over the sorted (key, value) rows, excluding
    // rows whose key embeds an epoch above STATE_AUDIT_MAX_EPOCH is not attempted — the
    // caller compares copies taken at the same reward cursor.
    if let Ok(path) = std::env::var("STATE_AUDIT_PREFIXES") {
        use sha2::Digest;
        use std::io::Write;
        let mut by: BTreeMap<String, (u64, sha2::Sha256)> = BTreeMap::new();
        store.for_each_meta_prefix("", |key, val| {
            let pre = key.split(':').next().unwrap_or(key).to_owned();
            let e = by.entry(pre).or_insert_with(|| (0, sha2::Sha256::new()));
            e.0 += 1; e.1.update(key.as_bytes()); e.1.update([0u8]); e.1.update(val); e.1.update([1u8]);
        });
        let mut f = std::io::BufWriter::new(std::fs::File::create(&path)?);
        for (p, (n, h)) in by { writeln!(f, "{p},{n},{}", hex::encode(h.finalize()))?; }
    }
    let mut sys = vec![hone_types::RECYCLE_FUND_ACCOUNT.to_string(), hone_types::TREASURY_ACCOUNT.to_string(), hone_types::TESTNET_FUND_ACCOUNT.to_string(), "treasury".into()];
    sys.extend(accounts.iter().cloned());
    let balances: BTreeMap<String, u64> = sys.iter()
        .map(|a| (a.clone(), store.get_balance(a, hone_types::NATIVE_TOKEN))).collect();
    let rows: Vec<serde_json::Value> = agg.iter().map(|((a, r), (n, sum, lo, hi))| serde_json::json!({
        "account": a, "role": r, "count": n, "sum_hunits": sum.to_string(), "first_epoch": lo, "last_epoch": hi,
    })).collect();
    let storage: BTreeMap<String, serde_json::Value> = per_epoch.into_iter().map(|(a, v)| {
        let total: u128 = v.iter().map(|(_, x)| *x as u128).sum();
        (a, serde_json::json!({ "entries": v.len(), "total_hunits": total.to_string(),
            "first": v.first().map(|x| x.0), "last": v.last().map(|x| x.0) }))
    }).collect();
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "balance_root": store.balance_merkle_root(), "balances": balances,
        "storage_rewards": storage, "roles": rows,
    }))?);
    Ok(())
}

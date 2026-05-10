//! On-chain key-value memory store: CF_META `memory:{account}:{key}`.

use anyhow::Result;
use crate::chain::Chain;

fn memory_key(account: &str, key: &str) -> String {
    format!("memory:{}:{}", account, key)
}

pub fn apply_set(chain: &Chain, account: &str, key: &str, value: &str) -> Result<()> {
    anyhow::ensure!(!key.is_empty(), "memory key cannot be empty");
    anyhow::ensure!(value.len() <= 4096, "memory value exceeds 4096 byte limit");
    chain.store.state_set(&memory_key(account, key), value.as_bytes())?;
    Ok(())
}

pub fn apply_delete(chain: &Chain, account: &str, key: &str) -> Result<()> {
    chain.store.state_delete(&memory_key(account, key))?;
    Ok(())
}

pub fn get(chain: &Chain, account: &str, key: &str) -> Option<String> {
    chain.store.state_get(&memory_key(account, key))
        .and_then(|b| String::from_utf8(b).ok())
}

pub fn scan(chain: &Chain, account: &str) -> Vec<(String, String)> {
    let prefix = format!("memory:{}:", account);
    chain.store.state_scan_prefix(&prefix)
        .into_iter()
        .filter_map(|(k, v)| {
            let key = k.strip_prefix(&prefix)?.to_string();
            let val = String::from_utf8(v).ok()?;
            Some((key, val))
        })
        .collect()
}

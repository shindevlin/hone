//! On-chain key-value memory store: CF_META `memory:{account}:{key}`.

#![allow(dead_code)]
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

/// Alias for `get` — returns the stored value for the given account+key.
pub fn get_entry(chain: &Chain, account: &str, key: &str) -> Option<String> {
    get(chain, account, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chain(label: &str) -> (Chain, tempfile::TempDir) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("btcpc_test_{}_", label))
            .tempdir()
            .unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(store, format!("node-{}", label), "btcpc-test".to_string());
        (chain, dir)
    }

    #[test]
    fn test_set_stores_value() {
        let (chain, _dir) = make_chain("mem_set");
        apply_set(&chain, "alice", "greeting", "hello").unwrap();
        assert_eq!(get_entry(&chain, "alice", "greeting"), Some("hello".to_string()));
    }

    #[test]
    fn test_delete_removes_value() {
        let (chain, _dir) = make_chain("mem_del");
        apply_set(&chain, "alice", "greeting", "hello").unwrap();
        apply_delete(&chain, "alice", "greeting").unwrap();
        assert_eq!(get_entry(&chain, "alice", "greeting"), None);
    }

    #[test]
    fn test_set_then_delete_returns_none() {
        let (chain, _dir) = make_chain("mem_setdel");
        apply_set(&chain, "bob", "key1", "value1").unwrap();
        apply_delete(&chain, "bob", "key1").unwrap();
        assert_eq!(get_entry(&chain, "bob", "key1"), None);
    }

    #[test]
    fn test_set_overwrites_existing() {
        let (chain, _dir) = make_chain("mem_overwrite");
        apply_set(&chain, "alice", "counter", "1").unwrap();
        apply_set(&chain, "alice", "counter", "2").unwrap();
        assert_eq!(get_entry(&chain, "alice", "counter"), Some("2".to_string()));
    }
}

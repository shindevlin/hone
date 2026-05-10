//! Peer commerce — CF_META scan for P2P storefront discovery.
//! P5-G: ~120 LOC

use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::chain::Chain;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Storefront {
    pub account: String,
    pub name: String,
    pub description: String,
    pub contact: Option<String>,
    /// BTCPC-FS CID pointing to the catalogue JSON.
    pub catalogue_cid: Option<String>,
    pub registered_epoch: u64,
}

fn storefront_key(account: &str) -> String { format!("storefront:{}", account) }

pub fn register(
    chain: &Chain,
    account: &str,
    name: &str,
    description: &str,
    contact: Option<&str>,
    catalogue_cid: Option<&str>,
    epoch: u64,
) -> Result<()> {
    let sf = Storefront {
        account: account.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        contact: contact.map(str::to_string),
        catalogue_cid: catalogue_cid.map(str::to_string),
        registered_epoch: epoch,
    };
    chain.store.state_set(&storefront_key(account), &serde_json::to_vec(&sf)?)?;
    Ok(())
}

pub fn deregister(chain: &Chain, account: &str) -> Result<()> {
    chain.store.state_delete(&storefront_key(account))?;
    Ok(())
}

pub fn get(chain: &Chain, account: &str) -> Option<Storefront> {
    chain.store.state_get(&storefront_key(account))
        .and_then(|b| serde_json::from_slice(&b).ok())
}

pub fn list(chain: &Chain) -> Vec<Storefront> {
    chain.store.state_scan_prefix("storefront:")
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice(&v).ok())
        .collect()
}

/// Register a storefront from a JSON string.  Parses the JSON to extract
/// `name`, `description`, and optional `contact` / `catalogue_cid` fields,
/// then delegates to `register`.
pub fn apply_register(chain: &Chain, account: &str, storefront_json: &str, epoch: u64) -> Result<()> {
    let v: serde_json::Value = serde_json::from_str(storefront_json)
        .map_err(|e| anyhow::anyhow!("invalid storefront_json: {}", e))?;
    let name = v["name"].as_str().unwrap_or("").to_string();
    let description = v["description"].as_str().unwrap_or("").to_string();
    let contact = v["contact"].as_str().map(str::to_string);
    let catalogue_cid = v["catalogue_cid"].as_str().map(str::to_string);
    register(
        chain,
        account,
        &name,
        &description,
        contact.as_deref(),
        catalogue_cid.as_deref(),
        epoch,
    )
}

/// Alias for `get` — returns the Storefront for the given account.
pub fn get_storefront(chain: &Chain, account: &str) -> Option<Storefront> {
    get(chain, account)
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

    const SF_JSON: &str = r#"{"name":"Alice Shop","description":"sells stuff","contact":"alice@btcpc","catalogue_cid":"bafycat1"}"#;

    #[test]
    fn test_register_stores_storefront() {
        let (chain, _dir) = make_chain("pc_reg");
        apply_register(&chain, "alice", SF_JSON, 1).unwrap();
        let sf = get_storefront(&chain, "alice").expect("storefront should exist");
        assert_eq!(sf.name, "Alice Shop");
        assert_eq!(sf.account, "alice");
    }

    #[test]
    fn test_re_register_updates_storefront() {
        let (chain, _dir) = make_chain("pc_update");
        apply_register(&chain, "alice", SF_JSON, 1).unwrap();
        let updated = r#"{"name":"Alice New Shop","description":"sells more stuff"}"#;
        apply_register(&chain, "alice", updated, 2).unwrap();
        let sf = get_storefront(&chain, "alice").expect("storefront");
        assert_eq!(sf.name, "Alice New Shop");
    }

    #[test]
    fn test_get_storefront_returns_none_for_unknown() {
        let (chain, _dir) = make_chain("pc_unknown");
        assert!(get_storefront(&chain, "nobody").is_none());
    }
}

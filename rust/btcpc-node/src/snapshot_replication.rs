//! Snapshot replication — slug-based chain snapshot save/load via BTCPC-FS.
//! P5-J: ~215 LOC

use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::chain::Chain;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub account: String,
    pub slug: String,
    /// BTCPC-FS CID of the snapshot data.
    pub cid: String,
    pub saved_epoch: u64,
}

fn snapshot_key(account: &str, slug: &str) -> String {
    format!("snapshot:{}:{}", account, slug)
}

pub fn apply_save(
    chain: &Chain,
    account: &str,
    slug: &str,
    cid: &str,
    epoch: u64,
) -> Result<()> {
    anyhow::ensure!(!slug.is_empty(), "snapshot slug cannot be empty");
    anyhow::ensure!(
        slug.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_'),
        "snapshot slug must be alphanumeric with dashes/underscores"
    );
    let snap = Snapshot {
        account: account.to_string(),
        slug: slug.to_string(),
        cid: cid.to_string(),
        saved_epoch: epoch,
    };
    chain.store.state_set(&snapshot_key(account, slug), &serde_json::to_vec(&snap)?)?;
    Ok(())
}

pub fn get(chain: &Chain, account: &str, slug: &str) -> Option<Snapshot> {
    chain.store.state_get(&snapshot_key(account, slug))
        .and_then(|b| serde_json::from_slice(&b).ok())
}

pub fn list(chain: &Chain, account: &str) -> Vec<Snapshot> {
    let prefix = format!("snapshot:{}:", account);
    chain.store.state_scan_prefix(&prefix)
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice(&v).ok())
        .collect()
}

pub fn delete(chain: &Chain, account: &str, slug: &str) -> Result<()> {
    chain.store.state_delete(&snapshot_key(account, slug))?;
    Ok(())
}

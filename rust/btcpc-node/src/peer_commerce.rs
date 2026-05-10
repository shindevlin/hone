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

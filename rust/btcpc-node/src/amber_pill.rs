//! AmberPill soulbound NFT — minted once per hardware fingerprint.
//! Holders receive a 1.5× entry weight multiplier.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use crate::chain::Chain;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmberPill {
    pub account: String,
    pub pill_id: String,
    pub minted_epoch: u64,
}

fn pill_key(account: &str) -> String { format!("amber_pill:{}", account) }
fn pill_id_key(pill_id: &str) -> String { format!("amber_pill_id:{}", pill_id) }

pub fn apply_mint(chain: &Chain, account: &str, pill_id: &str, epoch: u64) -> Result<()> {
    anyhow::ensure!(
        chain.store.state_get(&pill_key(account)).is_none(),
        "account '{}' already holds an AmberPill", account
    );
    anyhow::ensure!(
        chain.store.state_get(&pill_id_key(pill_id)).is_none(),
        "pill_id '{}' already minted", pill_id
    );
    let pill = AmberPill {
        account: account.to_string(),
        pill_id: pill_id.to_string(),
        minted_epoch: epoch,
    };
    chain.store.state_set(&pill_key(account), &serde_json::to_vec(&pill)?)?;
    chain.store.state_set(&pill_id_key(pill_id), account.as_bytes())?;
    Ok(())
}

pub fn has_pill(chain: &Chain, account: &str) -> bool {
    chain.store.state_get(&pill_key(account)).is_some()
}

pub fn get_pill(chain: &Chain, account: &str) -> Option<AmberPill> {
    chain.store.state_get(&pill_key(account))
        .and_then(|b| serde_json::from_slice(&b).ok())
}

/// Entry weight multiplier for AmberPill holders: 1.5× (expressed as BPS: 15_000 / 10_000).
pub const AMBER_PILL_WEIGHT_BPS: u64 = 15_000;

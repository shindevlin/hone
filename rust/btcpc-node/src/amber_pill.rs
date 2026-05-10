//! AmberPill soulbound NFT — minted once per hardware fingerprint.
//! Holders receive a 1.5× entry weight multiplier.

#![allow(dead_code)]
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

/// Look up an AmberPill by its pill_id string (not by account).
pub fn get_pill_by_id(chain: &Chain, pill_id: &str) -> Option<AmberPill> {
    let account = chain.store.state_get(&pill_id_key(pill_id))
        .and_then(|b| String::from_utf8(b).ok())?;
    get_pill(chain, &account)
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
    fn test_mint_stores_pill() {
        let (chain, _dir) = make_chain("amber_mint");
        apply_mint(&chain, "alice", "pill-abc", 1).unwrap();
        let pill = get_pill(&chain, "alice").expect("pill should exist");
        assert_eq!(pill.pill_id, "pill-abc");
        assert_eq!(pill.minted_epoch, 1);
    }

    #[test]
    fn test_second_mint_same_pill_id_errors() {
        // The existing code returns Err on duplicate pill_id — this pins that behaviour.
        let (chain, _dir) = make_chain("amber_dup_pill");
        apply_mint(&chain, "alice", "pill-dup", 1).unwrap();
        let result = apply_mint(&chain, "bob", "pill-dup", 2);
        assert!(result.is_err(), "duplicate pill_id must be rejected");
    }

    #[test]
    fn test_second_mint_same_account_errors() {
        let (chain, _dir) = make_chain("amber_dup_acc");
        apply_mint(&chain, "alice", "pill-x", 1).unwrap();
        let result = apply_mint(&chain, "alice", "pill-y", 2);
        assert!(result.is_err(), "account already holds a pill");
    }

    #[test]
    fn test_pill_owner_matches_account() {
        let (chain, _dir) = make_chain("amber_owner");
        apply_mint(&chain, "carol", "pill-carol", 5).unwrap();
        let pill = get_pill_by_id(&chain, "pill-carol").expect("pill by id");
        assert_eq!(pill.account, "carol");
    }
}

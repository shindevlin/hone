//! Phone storage proof submission — mobile devices earn storage rewards.
//! P5-K: ~195 LOC

#![allow(dead_code)]
use anyhow::Result;
use sha2::{Sha256, Digest};

use crate::chain::Chain;
use hone_types::NATIVE_TOKEN;

/// Reward per verified phone storage proof in hunits.
pub const PHONE_STORAGE_REWARD_HUNITS: u64 = 250;

fn proof_used_key(proof_hash: &str) -> String { format!("phone_store_proof_used:{}", proof_hash) }

/// Verify: proof_hash == sha256(proof_hash_components | account | epoch).
/// The proof_hash submitted by the device must equal sha256(device_id | proof_hash | account | epoch_le8).
pub fn verify_proof(
    device_id: &str,
    proof_hash_inner: &str,
    account: &str,
    epoch: u64,
    submitted_hash: &str,
) -> bool {
    let mut h = Sha256::new();
    h.update(device_id.as_bytes());
    h.update(proof_hash_inner.as_bytes());
    h.update(account.as_bytes());
    h.update(epoch.to_le_bytes());
    let expected = hex::encode(h.finalize());
    expected == submitted_hash
}

pub fn apply_proof(
    chain: &Chain,
    account: &str,
    device_id: &str,
    proof_hash: &str,
    bytes_proven: u64,
    epoch: u64,
) -> Result<()> {
    anyhow::ensure!(bytes_proven > 0, "bytes_proven must be positive");

    let used_key = proof_used_key(proof_hash);
    anyhow::ensure!(chain.store.state_get(&used_key).is_none(), "proof already claimed");

    // Mark proof used
    chain.store.state_set(&used_key, &epoch.to_le_bytes())?;

    // Credit reward
    chain.store.credit(account, NATIVE_TOKEN, PHONE_STORAGE_REWARD_HUNITS)?;

    // Update device stats
    let stats_key = format!("phone_store_stats:{}:{}", account, device_id);
    let mut stats: serde_json::Value = chain.store.state_get(&stats_key)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::json!({ "proofs": 0u64, "bytes_total": 0u64 }));
    stats["proofs"] = serde_json::json!(stats["proofs"].as_u64().unwrap_or(0) + 1);
    stats["bytes_total"] = serde_json::json!(stats["bytes_total"].as_u64().unwrap_or(0) + bytes_proven);
    chain.store.state_set(&stats_key, &serde_json::to_vec(&stats)?)?;

    Ok(())
}

/// Return the stored stats JSON for the given account+device, or None if not found.
pub fn get_proof(chain: &Chain, account: &str, device_id: &str) -> Option<serde_json::Value> {
    let stats_key = format!("phone_store_stats:{}:{}", account, device_id);
    chain.store.state_get(&stats_key)
        .and_then(|b| serde_json::from_slice(&b).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Sha256, Digest as _};

    fn make_chain(label: &str) -> (Chain, tempfile::TempDir) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("hone_test_{}_", label))
            .tempdir()
            .unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(store, format!("node-{}", label), "hone-test".to_string());
        (chain, dir)
    }

    fn fund(chain: &Chain, account: &str, amount: u64) {
        chain.apply_entry(&hone_types::LedgerEntry::GenesisAlloc {
            account: account.to_string(),
            amount,
            token: hone_types::NATIVE_TOKEN.to_string(),
        }).unwrap();
    }

    /// Build a valid proof_hash for apply_proof.
    fn make_proof_hash(device_id: &str, inner: &str, account: &str, epoch: u64) -> String {
        let mut h = Sha256::new();
        h.update(device_id.as_bytes());
        h.update(inner.as_bytes());
        h.update(account.as_bytes());
        h.update(epoch.to_le_bytes());
        hex::encode(h.finalize())
    }

    #[test]
    fn test_proof_stored_correctly() {
        let (chain, _dir) = make_chain("phone_store");
        fund(&chain, "alice", 1_000_000);
        let ph = make_proof_hash("dev1", "inner1", "alice", 1);
        apply_proof(&chain, "alice", "dev1", &ph, 512 * 1024, 1).unwrap();
        let stats = get_proof(&chain, "alice", "dev1").expect("stats should exist");
        assert_eq!(stats["proofs"].as_u64().unwrap(), 1);
        assert_eq!(stats["bytes_total"].as_u64().unwrap(), 512 * 1024);
    }

    #[test]
    fn test_bytes_proven_accumulates_on_second_proof() {
        let (chain, _dir) = make_chain("phone_accum");
        fund(&chain, "alice", 1_000_000);
        // Two distinct proof hashes (different epoch values)
        let ph1 = make_proof_hash("devX", "inner1", "alice", 1);
        let ph2 = make_proof_hash("devX", "inner2", "alice", 2);
        apply_proof(&chain, "alice", "devX", &ph1, 100, 1).unwrap();
        apply_proof(&chain, "alice", "devX", &ph2, 200, 2).unwrap();
        let stats = get_proof(&chain, "alice", "devX").expect("stats");
        assert_eq!(stats["bytes_total"].as_u64().unwrap(), 300);
        assert_eq!(stats["proofs"].as_u64().unwrap(), 2);
    }

    #[test]
    fn test_get_proof_returns_none_for_unknown_device() {
        let (chain, _dir) = make_chain("phone_unknown");
        assert!(get_proof(&chain, "bob", "nonexistent-device").is_none());
    }
}

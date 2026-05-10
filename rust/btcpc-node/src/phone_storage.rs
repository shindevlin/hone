//! Phone storage proof submission — mobile devices earn storage rewards.
//! P5-K: ~195 LOC

use anyhow::Result;
use sha2::{Sha256, Digest};

use crate::chain::Chain;
use btcpc_types::NATIVE_TOKEN;

/// Reward per verified phone storage proof in dreams.
pub const PHONE_STORAGE_REWARD_DREAMS: u64 = 250;

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
    chain.store.credit(account, NATIVE_TOKEN, PHONE_STORAGE_REWARD_DREAMS)?;

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

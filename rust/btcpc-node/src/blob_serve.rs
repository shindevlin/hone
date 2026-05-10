//! Blob serve proof submission and reward path.
//! P5-E: ~225 LOC

use anyhow::Result;
use sha2::{Sha256, Digest};

use crate::chain::Chain;
use btcpc_types::NATIVE_TOKEN;

/// Reward per verified blob serve proof in dreams (0.01 BTCPC).
pub const BLOB_SERVE_REWARD_DREAMS: u64 = 1_000_000;

/// Maximum bytes_served per proof (prevents overclaiming).
pub const MAX_BYTES_PER_PROOF: u64 = 100 * 1024 * 1024 * 1024; // 100 GB

fn proof_used_key(proof_hash: &str) -> String { format!("blob_proof_used:{}", proof_hash) }

/// Verify that `proof_hash == sha256(cid | bytes_served_le8 | node_id | epoch_le8)`.
pub fn verify_proof(cid: &str, bytes_served: u64, node_id: &str, epoch: u64, proof_hash: &str) -> bool {
    let mut h = Sha256::new();
    h.update(cid.as_bytes());
    h.update(bytes_served.to_le_bytes());
    h.update(node_id.as_bytes());
    h.update(epoch.to_le_bytes());
    let expected = hex::encode(h.finalize());
    expected == proof_hash
}

pub fn apply_serve_proof(
    chain: &Chain,
    node_id: &str,
    cid: &str,
    bytes_served: u64,
    proof_hash: &str,
    epoch: u64,
) -> Result<()> {
    anyhow::ensure!(bytes_served > 0, "bytes_served must be positive");
    anyhow::ensure!(bytes_served <= MAX_BYTES_PER_PROOF, "bytes_served exceeds per-proof limit");

    anyhow::ensure!(
        !verify_proof(cid, bytes_served, node_id, epoch, proof_hash) ||
        chain.store.state_get(&proof_used_key(proof_hash)).is_none(),
        "proof already claimed"
    );

    anyhow::ensure!(
        verify_proof(cid, bytes_served, node_id, epoch, proof_hash),
        "invalid proof hash"
    );

    // Mark proof as used
    chain.store.state_set(&proof_used_key(proof_hash), &epoch.to_le_bytes())?;

    // Credit reward
    let reward = BLOB_SERVE_REWARD_DREAMS;
    chain.store.credit(node_id, NATIVE_TOKEN, reward)?;

    // Update serve stats
    let stats_key = format!("blob_serve_stats:{}", node_id);
    let mut stats: serde_json::Value = chain.store.state_get(&stats_key)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::json!({ "proofs": 0u64, "bytes_total": 0u64 }));
    stats["proofs"] = serde_json::json!(stats["proofs"].as_u64().unwrap_or(0) + 1);
    stats["bytes_total"] = serde_json::json!(stats["bytes_total"].as_u64().unwrap_or(0) + bytes_served);
    chain.store.state_set(&stats_key, &serde_json::to_vec(&stats)?)?;

    Ok(())
}

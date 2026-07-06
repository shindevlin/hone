//! Hive external replica accounting for HONE-FS.
//!
//! Hive is treated as a decentralized external storage domain, not as local disk.
//! A storage operator first commits a Hive custom_json reference, then an independent
//! HONE verifier attests that the Hive payload was fetched and matched the HONE-FS
//! CID/Merkle metadata. Only verified replicas add a second storage-domain reward slot.

use anyhow::{bail, ensure, Result};
use sha2::{Digest, Sha256};

use hone_types::LedgerEntry;

use crate::chain::Chain;

/// Hive tx refs must be reasonably confirmed before verification can count.
pub const HIVE_REPLICA_MIN_CONFIRMATIONS: u32 = 20;
/// A single proof cannot claim more than 100 GiB.
pub const HIVE_REPLICA_MAX_BYTES_PER_PROOF: u64 = 100 * 1024 * 1024 * 1024;
/// Manifest-only records are useful witnesses, but are not full data replicas.
pub const HIVE_MANIFEST_MAX_SCORE_BYTES: u64 = 1024 * 1024;
/// One Hive domain slot per operator per epoch. Multiple verified refs aggregate here.
pub const HIVE_REPLICA_MAX_SCORE_BYTES_PER_EPOCH: u64 = 100 * 1024 * 1024 * 1024;

const HIVE_FULL_BPS: u64 = 7_500;
const HIVE_CHUNK_BPS: u64 = 5_000;
const HIVE_PARITY_BPS: u64 = 3_000;
const HIVE_MANIFEST_BPS: u64 = 500;

pub fn apply_commit(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::HiveReplicaCommit {
        node_id,
        cid,
        hive_account,
        custom_json_id,
        hive_block_num,
        hive_tx_id,
        op_index,
        payload_sha256,
        merkle_root,
        bytes_replicated,
        replica_kind,
        confirmations,
        epoch,
        signed_by,
        ..
    } = entry
    else {
        bail!("expected HiveReplicaCommit");
    };

    ensure!(
        signed_by == node_id,
        "HiveReplicaCommit must be signed by node_id"
    );
    validate_common(
        cid,
        hive_account,
        custom_json_id,
        *hive_block_num,
        hive_tx_id,
        payload_sha256,
        merkle_root,
        *bytes_replicated,
        replica_kind,
    )?;
    ensure!(
        *confirmations >= HIVE_REPLICA_MIN_CONFIRMATIONS,
        "Hive replica requires at least {} confirmations",
        HIVE_REPLICA_MIN_CONFIRMATIONS
    );

    let ref_id = replica_ref_id(
        cid,
        hive_account,
        custom_json_id,
        *hive_block_num,
        hive_tx_id,
        *op_index,
        payload_sha256,
        merkle_root,
    );
    let key = format!("hive_replica_commit:{}", ref_id);
    if let Some(existing) = chain.store.state_get(&key) {
        let existing: serde_json::Value = serde_json::from_slice(&existing).unwrap_or_default();
        ensure!(
            existing["node_id"].as_str() == Some(node_id.as_str()),
            "Hive replica ref already committed by another node"
        );
        bail!("Hive replica ref already committed");
    }

    let value = serde_json::json!({
        "ref_id": ref_id,
        "node_id": node_id,
        "cid": cid,
        "hive_account": hive_account,
        "custom_json_id": custom_json_id,
        "hive_block_num": hive_block_num,
        "hive_tx_id": hive_tx_id,
        "op_index": op_index,
        "payload_sha256": payload_sha256,
        "merkle_root": merkle_root,
        "bytes_replicated": bytes_replicated,
        "replica_kind": replica_kind,
        "confirmations": confirmations,
        "epoch": epoch,
        "storage_domain": "hive",
    });
    chain.store.state_set(&key, &serde_json::to_vec(&value)?)?;
    Ok(())
}

pub fn apply_verify(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::HiveReplicaVerify {
        verifier,
        node_id,
        cid,
        hive_account,
        custom_json_id,
        hive_block_num,
        hive_tx_id,
        op_index,
        payload_sha256,
        merkle_root,
        bytes_verified,
        replica_kind,
        challenge_hash,
        epoch,
        signed_by,
        ..
    } = entry
    else {
        bail!("expected HiveReplicaVerify");
    };

    ensure!(
        signed_by == verifier,
        "HiveReplicaVerify must be signed by verifier"
    );
    ensure!(
        verifier != node_id,
        "Hive replica verifier must be independent from node_id"
    );
    validate_common(
        cid,
        hive_account,
        custom_json_id,
        *hive_block_num,
        hive_tx_id,
        payload_sha256,
        merkle_root,
        *bytes_verified,
        replica_kind,
    )?;
    ensure!(
        expected_challenge_hash(chain, node_id, cid, hive_tx_id, *epoch).as_deref()
            == Some(challenge_hash.as_str()),
        "invalid Hive replica challenge_hash"
    );

    let ref_id = replica_ref_id(
        cid,
        hive_account,
        custom_json_id,
        *hive_block_num,
        hive_tx_id,
        *op_index,
        payload_sha256,
        merkle_root,
    );
    let commit_key = format!("hive_replica_commit:{}", ref_id);
    let commit_bytes = chain
        .store
        .state_get(&commit_key)
        .ok_or_else(|| anyhow::anyhow!("Hive replica commit not found"))?;
    let commit: serde_json::Value = serde_json::from_slice(&commit_bytes)?;
    ensure!(
        commit["node_id"].as_str() == Some(node_id.as_str()),
        "commit node_id mismatch"
    );
    ensure!(
        commit["bytes_replicated"].as_u64() == Some(*bytes_verified),
        "verified bytes mismatch"
    );
    ensure!(
        commit["replica_kind"].as_str() == Some(replica_kind.as_str()),
        "replica kind mismatch"
    );

    let rewarded_key = format!("hive_replica_rewarded:{}:{}", epoch, ref_id);
    ensure!(
        chain.store.state_get(&rewarded_key).is_none(),
        "Hive replica already rewarded this epoch"
    );
    chain.store.state_set(&rewarded_key, verifier.as_bytes())?;

    let score = replica_score_bytes(*bytes_verified, replica_kind)?;
    let beat_key = format!("storage_beat:{}:{}:hive", epoch, node_id);
    let mut beat: serde_json::Value = chain
        .store
        .state_get(&beat_key)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| {
            serde_json::json!({
                "node_id": node_id,
                "epoch": epoch,
                "bytes_proven": 0u64,
                "query_count": 0u64,
                "proof_valid": true,
                "tier": 1u8,
                "backend": "hive",
                "storage_domain": "hive",
                "replica_count": 0u64,
                "raw_bytes_verified": 0u64,
            })
        });

    let current_score = beat["bytes_proven"].as_u64().unwrap_or(0);
    let next_score = current_score
        .saturating_add(score)
        .min(HIVE_REPLICA_MAX_SCORE_BYTES_PER_EPOCH);
    let current_raw = beat["raw_bytes_verified"].as_u64().unwrap_or(0);
    beat["bytes_proven"] = serde_json::json!(next_score);
    beat["raw_bytes_verified"] = serde_json::json!(current_raw.saturating_add(*bytes_verified));
    beat["replica_count"] = serde_json::json!(beat["replica_count"].as_u64().unwrap_or(0) + 1);
    beat["last_ref_id"] = serde_json::json!(ref_id);
    beat["last_cid"] = serde_json::json!(cid);
    beat["last_hive_tx_id"] = serde_json::json!(hive_tx_id);
    beat["last_verifier"] = serde_json::json!(verifier);
    beat["last_replica_kind"] = serde_json::json!(replica_kind);
    chain
        .store
        .state_set(&beat_key, &serde_json::to_vec(&beat)?)?;

    Ok(())
}

pub fn replica_score_bytes(bytes: u64, replica_kind: &str) -> Result<u64> {
    ensure!(bytes > 0, "Hive replica bytes must be positive");
    ensure!(
        bytes <= HIVE_REPLICA_MAX_BYTES_PER_PROOF,
        "Hive replica bytes exceed per-proof limit"
    );
    let (basis_points, accountable_bytes) = match replica_kind {
        "full" => (HIVE_FULL_BPS, bytes),
        "chunk" => (HIVE_CHUNK_BPS, bytes),
        "parity" => (HIVE_PARITY_BPS, bytes),
        "manifest" => (HIVE_MANIFEST_BPS, bytes.min(HIVE_MANIFEST_MAX_SCORE_BYTES)),
        _ => bail!("unsupported Hive replica kind '{}'", replica_kind),
    };
    Ok(((accountable_bytes as u128 * basis_points as u128) / 10_000) as u64)
}

pub fn expected_challenge_hash(
    chain: &Chain,
    node_id: &str,
    cid: &str,
    hive_tx_id: &str,
    epoch: u64,
) -> Option<String> {
    let prev = chain
        .store
        .state_get(&format!("epoch_seal_hash:{}", epoch.saturating_sub(1)))?;
    let prev_seal_hash = String::from_utf8_lossy(&prev);
    let mut h = Sha256::new();
    h.update(
        format!(
            "{}:{}:hive:{}:{}:{}",
            prev_seal_hash, node_id, cid, hive_tx_id, epoch
        )
        .as_bytes(),
    );
    Some(hex::encode(h.finalize()))
}

pub fn replica_ref_id(
    cid: &str,
    hive_account: &str,
    custom_json_id: &str,
    hive_block_num: u64,
    hive_tx_id: &str,
    op_index: u32,
    payload_sha256: &str,
    merkle_root: &str,
) -> String {
    let mut h = Sha256::new();
    h.update(cid.as_bytes());
    h.update(hive_account.as_bytes());
    h.update(custom_json_id.as_bytes());
    h.update(hive_block_num.to_le_bytes());
    h.update(hive_tx_id.as_bytes());
    h.update(op_index.to_le_bytes());
    h.update(payload_sha256.as_bytes());
    h.update(merkle_root.as_bytes());
    hex::encode(h.finalize())
}

fn validate_common(
    cid: &str,
    hive_account: &str,
    custom_json_id: &str,
    hive_block_num: u64,
    hive_tx_id: &str,
    payload_sha256: &str,
    merkle_root: &str,
    bytes: u64,
    replica_kind: &str,
) -> Result<()> {
    ensure!(is_safe_token(cid, 1, 256), "invalid CID");
    ensure!(is_hive_account(hive_account), "invalid Hive account");
    ensure!(
        is_safe_token(custom_json_id, 1, 128),
        "invalid Hive custom_json id"
    );
    ensure!(hive_block_num > 0, "Hive block number must be positive");
    ensure!(
        is_hex_len(hive_tx_id, 40) || is_hex_len(hive_tx_id, 64),
        "invalid Hive tx id"
    );
    ensure!(is_hex_len(payload_sha256, 64), "invalid payload_sha256");
    ensure!(is_hex_len(merkle_root, 64), "invalid merkle_root");
    ensure!(bytes > 0, "Hive replica bytes must be positive");
    ensure!(
        bytes <= HIVE_REPLICA_MAX_BYTES_PER_PROOF,
        "Hive replica bytes exceed per-proof limit"
    );
    let _ = replica_score_bytes(bytes, replica_kind)?;
    Ok(())
}

fn is_safe_token(value: &str, min: usize, max: usize) -> bool {
    let len = value.len();
    len >= min
        && len <= max
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b':' | b'/'))
}

fn is_hive_account(value: &str) -> bool {
    let len = value.len();
    len >= 3
        && len <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'-'))
}

fn is_hex_len(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chain(label: &str) -> (Chain, tempfile::TempDir) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("hone_hive_replica_{}_", label))
            .tempdir()
            .unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(store, format!("node-{}", label), "hone-test".to_string());
        chain
            .store
            .state_set("epoch_seal_hash:0", b"prev-seal")
            .unwrap();
        (chain, dir)
    }

    fn fund(chain: &Chain, account: &str) {
        chain
            .apply_entry(&LedgerEntry::GenesisAlloc {
                account: account.to_string(),
                amount: 1_000_000,
                token: hone_types::NATIVE_TOKEN.to_string(),
            })
            .unwrap();
    }

    fn commit(node_id: &str, kind: &str, bytes: u64) -> LedgerEntry {
        LedgerEntry::HiveReplicaCommit {
            node_id: node_id.to_string(),
            cid: "bafyhoneblob".to_string(),
            hive_account: "hone.hive".to_string(),
            custom_json_id: "hone_fs_v1".to_string(),
            hive_block_num: 42,
            hive_tx_id: "0123456789abcdef0123456789abcdef01234567".to_string(),
            op_index: 0,
            payload_sha256: "a".repeat(64),
            merkle_root: "b".repeat(64),
            bytes_replicated: bytes,
            replica_kind: kind.to_string(),
            confirmations: HIVE_REPLICA_MIN_CONFIRMATIONS,
            epoch: 1,
            nonce: 0,
            signed_by: node_id.to_string(),
            signature: None,
        }
    }

    fn verify(chain: &Chain, node_id: &str, verifier: &str, kind: &str, bytes: u64) -> LedgerEntry {
        let challenge_hash = expected_challenge_hash(
            chain,
            node_id,
            "bafyhoneblob",
            "0123456789abcdef0123456789abcdef01234567",
            1,
        )
        .unwrap();
        LedgerEntry::HiveReplicaVerify {
            verifier: verifier.to_string(),
            node_id: node_id.to_string(),
            cid: "bafyhoneblob".to_string(),
            hive_account: "hone.hive".to_string(),
            custom_json_id: "hone_fs_v1".to_string(),
            hive_block_num: 42,
            hive_tx_id: "0123456789abcdef0123456789abcdef01234567".to_string(),
            op_index: 0,
            payload_sha256: "a".repeat(64),
            merkle_root: "b".repeat(64),
            bytes_verified: bytes,
            replica_kind: kind.to_string(),
            challenge_hash,
            epoch: 1,
            nonce: 0,
            signed_by: verifier.to_string(),
            signature: None,
        }
    }

    #[test]
    fn verified_full_replica_creates_hive_storage_slot() {
        let (chain, _dir) = make_chain("full_slot");
        fund(&chain, "storer");
        fund(&chain, "verifier");

        apply_commit(&chain, &commit("storer", "full", 10_000)).unwrap();
        apply_verify(
            &chain,
            &verify(&chain, "storer", "verifier", "full", 10_000),
        )
        .unwrap();

        let raw = chain.store.state_get("storage_beat:1:storer:hive").unwrap();
        let beat: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(beat["backend"].as_str(), Some("hive"));
        assert_eq!(beat["bytes_proven"].as_u64(), Some(7_500));
        assert_eq!(beat["replica_count"].as_u64(), Some(1));
    }

    #[test]
    fn manifest_replica_is_capped_and_discounted() {
        let score = replica_score_bytes(10 * 1024 * 1024 * 1024, "manifest").unwrap();
        assert_eq!(
            score,
            HIVE_MANIFEST_MAX_SCORE_BYTES * HIVE_MANIFEST_BPS / 10_000
        );
    }

    #[test]
    fn self_verification_is_rejected() {
        let (chain, _dir) = make_chain("self_verify");
        fund(&chain, "storer");

        apply_commit(&chain, &commit("storer", "full", 10_000)).unwrap();
        let err =
            apply_verify(&chain, &verify(&chain, "storer", "storer", "full", 10_000)).unwrap_err();
        assert!(err.to_string().contains("independent"));
    }

    #[test]
    fn duplicate_ref_cannot_reward_twice_in_same_epoch() {
        let (chain, _dir) = make_chain("dupe_reward");
        fund(&chain, "storer");
        fund(&chain, "verifier");
        fund(&chain, "verifier2");

        apply_commit(&chain, &commit("storer", "full", 10_000)).unwrap();
        apply_verify(
            &chain,
            &verify(&chain, "storer", "verifier", "full", 10_000),
        )
        .unwrap();
        let err = apply_verify(
            &chain,
            &verify(&chain, "storer", "verifier2", "full", 10_000),
        )
        .unwrap_err();
        assert!(err.to_string().contains("already rewarded"));
    }
}

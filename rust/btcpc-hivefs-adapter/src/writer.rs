//! Phase 2 — Hive writer.
//!
//! Reads a local BTCPC-FS file, computes hashes, broadcasts to Hive as
//! `custom_json`, waits for MIN_CONFIRMATIONS, then submits HiveReplicaCommit
//! to the BTCPC node.

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::btcpc_client::{BtcpcClient, HiveCommitParams};
use crate::hive_client::{HiveClient, MIN_CONFIRMATIONS};

/// Environment-sourced writer configuration.
pub struct WriterConfig {
    pub hive_account: String,
    pub hive_posting_key_hex: String,
    pub btcpc_node_id: String,
    pub btcpc_posting_key_hex: String,
    pub btcpc_api_url: String,
    pub hive_api_url: String,
}

impl WriterConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            hive_account: std::env::var("HIVE_ACCOUNT")
                .context("HIVE_ACCOUNT not set")?,
            hive_posting_key_hex: std::env::var("HIVE_POSTING_KEY")
                .context("HIVE_POSTING_KEY not set")?,
            btcpc_node_id: std::env::var("BTCPC_NODE_ID")
                .context("BTCPC_NODE_ID not set")?,
            btcpc_posting_key_hex: std::env::var("BTCPC_POSTING_KEY")
                .context("BTCPC_POSTING_KEY not set")?,
            btcpc_api_url: std::env::var("BTCPC_API_URL")
                .unwrap_or_else(|_| "http://localhost:4242".into()),
            hive_api_url: std::env::var("HIVE_API_URL")
                .unwrap_or_else(|_| "https://api.hive.blog".into()),
        })
    }
}

/// Run the full Phase 2 write flow.
///
/// 1. Read file and compute SHA256 + Merkle root.
/// 2. Build Hive custom_json payload.
/// 3. Sign and broadcast to Hive.
/// 4. Wait for MIN_CONFIRMATIONS (20 blocks).
/// 5. Submit HiveReplicaCommit to the BTCPC node.
pub async fn run_write(
    cfg: WriterConfig,
    cid: String,
    file_path: String,
    kind: String,
) -> Result<()> {
    // ── 1. Read file ──────────────────────────────────────────────────────────
    let path = Path::new(&file_path);
    let file_bytes = std::fs::read(path)
        .with_context(|| format!("failed to read {}", file_path))?;
    let file_len = file_bytes.len() as u64;

    let payload_sha256 = sha256_hex(&file_bytes);
    // For single-file replicas, merkle_root == payload_sha256 (single-leaf Merkle tree).
    let merkle_root = payload_sha256.clone();

    eprintln!("file: {} bytes, sha256={}", file_len, &payload_sha256[..16]);

    // ── 2. Build custom_json payload ─────────────────────────────────────────
    let hive_json_payload = build_hive_payload(&cid, &kind, &file_bytes, &payload_sha256, &merkle_root)?;
    eprintln!("hive payload type: {}", hive_json_payload["type"].as_str().unwrap_or("?"));

    // ── 3. Broadcast to Hive ─────────────────────────────────────────────────
    let hive = HiveClient::new(&cfg.hive_api_url);
    let props = hive.get_dynamic_global_properties().await?;
    let target_block = props.head_block_number;
    eprintln!("Hive head block: {}", target_block);

    let mut tx = hive.build_custom_json_tx(
        &props,
        &cfg.hive_account,
        "btcpc_fs_v1",
        &hive_json_payload,
    );
    HiveClient::sign_tx_placeholder(&mut tx, &cfg.hive_posting_key_hex)?;

    let hive_tx_id = hive.broadcast_transaction(&tx).await?;
    eprintln!("Hive tx broadcast: {}", hive_tx_id);

    // ── 4. Wait for MIN_CONFIRMATIONS ────────────────────────────────────────
    eprintln!("waiting for {} Hive confirmations…", MIN_CONFIRMATIONS);
    hive.wait_for_confirmations(target_block).await?;

    // Block number the tx landed in = target_block + 1 (conservative; the tx
    // was broadcast after we read the head block, so it lands in target+1 or later).
    let hive_block_num = target_block + 1;
    // op_index is always 0: we submit one operation per transaction.
    let op_index: u32 = 0;
    let confirmations = MIN_CONFIRMATIONS;

    eprintln!("confirmed in block ~{}", hive_block_num);

    // ── 5. Submit HiveReplicaCommit ───────────────────────────────────────────
    let btcpc = BtcpcClient::new(&cfg.btcpc_api_url);
    let commit_params = HiveCommitParams {
        node_id: cfg.btcpc_node_id.clone(),
        cid: cid.clone(),
        hive_account: cfg.hive_account.clone(),
        custom_json_id: "btcpc_fs_v1".into(),
        hive_block_num,
        hive_tx_id: hive_tx_id.clone(),
        op_index,
        payload_sha256: payload_sha256.clone(),
        merkle_root: merkle_root.clone(),
        bytes_replicated: file_len,
        replica_kind: kind.clone(),
        confirmations,
    };

    let resp = btcpc
        .post_hive_commit(commit_params, &cfg.btcpc_posting_key_hex)
        .await?;

    eprintln!("HiveReplicaCommit submitted: {}", resp);
    println!(
        "{{\"ok\":true,\"hive_tx_id\":\"{}\",\"hive_block_num\":{},\"cid\":\"{}\"}}",
        hive_tx_id, hive_block_num, cid
    );
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

/// Build the Hive custom_json body per the BTCPC-FS spec.
///
/// For `manifest` and `full` kinds the payload is stored as the SHA256 of the
/// JSON file bytes. For `chunk` the payload bytes are included as base64 if the
/// encoded size fits within Hive's ~8 KiB custom_json budget; otherwise only
/// the hash is stored (the chain records the hash, not the bytes).
fn build_hive_payload(
    cid: &str,
    kind: &str,
    file_bytes: &[u8],
    payload_sha256: &str,
    merkle_root: &str,
) -> Result<serde_json::Value> {
    const MAX_B64_BYTES: usize = 6_000; // conservative Hive custom_json size budget

    match kind {
        "manifest" => {
            // Parse the manifest JSON to extract metadata.
            let manifest: serde_json::Value = serde_json::from_slice(file_bytes)
                .context("manifest file must be valid JSON")?;
            Ok(serde_json::json!({
                "type": "btcpc_fs_manifest_v1",
                "cid": cid,
                "size_bytes": manifest.get("size_bytes").cloned().unwrap_or(serde_json::Value::Null),
                "chunk_size": manifest.get("chunk_size").cloned().unwrap_or(serde_json::Value::Null),
                "chunk_count": manifest.get("chunk_count").cloned().unwrap_or(serde_json::Value::Null),
                "merkle_root": merkle_root,
                "encrypted": manifest.get("encrypted").cloned().unwrap_or(serde_json::json!(false)),
            }))
        }
        "chunk" => {
            let b64_data = B64.encode(file_bytes);
            // NOTE: Hive custom_json is limited to ~8 KiB. Chunks larger than
            // MAX_B64_BYTES will be recorded by hash only. The Merkle root still
            // proves data integrity for the BTCPC verifier.
            let payload_b64 = if b64_data.len() <= MAX_B64_BYTES {
                Some(b64_data)
            } else {
                eprintln!(
                    "  chunk payload too large for Hive custom_json ({} bytes encoded), omitting payload_b64",
                    b64_data.len()
                );
                None
            };
            let mut v = serde_json::json!({
                "type": "btcpc_fs_chunk_v1",
                "cid": cid,
                "chunk_index": 0,
                "payload_sha256": payload_sha256,
                "merkle_root": merkle_root,
            });
            if let Some(b64) = payload_b64 {
                v["payload_b64"] = serde_json::Value::String(b64);
            }
            Ok(v)
        }
        "parity" => {
            Ok(serde_json::json!({
                "type": "btcpc_fs_parity_v1",
                "cid": cid,
                "payload_sha256": payload_sha256,
                "merkle_root": merkle_root,
            }))
        }
        "full" => {
            Ok(serde_json::json!({
                "type": "btcpc_fs_manifest_v1",
                "cid": cid,
                "merkle_root": merkle_root,
                "payload_sha256": payload_sha256,
            }))
        }
        other => bail!("unsupported replica kind '{}'; expected full|chunk|parity|manifest", other),
    }
}

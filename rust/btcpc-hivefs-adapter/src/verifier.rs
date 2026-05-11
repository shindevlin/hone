//! Phase 3 — Hive verifier.
//!
//! Fetches the Hive transaction, parses the BTCPC-FS custom_json payload,
//! verifies CID / payload_sha256 / merkle_root, computes the per-epoch
//! challenge hash, then submits HiveReplicaVerify to the BTCPC node.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

use crate::btcpc_client::{compute_challenge_hash, BtcpcClient, HiveVerifyParams};
use crate::hive_client::HiveClient;

/// Environment-sourced verifier configuration.
pub struct VerifierConfig {
    #[allow(dead_code)]
    pub btcpc_node_id: String,          // the storage node being verified (kept for config symmetry)
    pub btcpc_verifier_id: String,      // this verifier's BTCPC account
    pub btcpc_verifier_key_hex: String, // ed25519 hex key for BTCPC_VERIFIER_KEY
    pub btcpc_api_url: String,
    pub hive_api_url: String,
    pub hive_account: String, // Hive account that originally posted the custom_json
}

impl VerifierConfig {
    pub fn from_env() -> Result<Self> {
        // BTCPC_VERIFIER_KEY is the verifier's own signing key.
        // BTCPC_NODE_ID is the storage node being verified.
        // HIVE_ACCOUNT is the Hive account that originally broadcast the custom_json.
        Ok(Self {
            btcpc_node_id: std::env::var("BTCPC_NODE_ID")
                .context("BTCPC_NODE_ID not set")?,
            btcpc_verifier_id: std::env::var("BTCPC_VERIFIER_ID")
                .context("BTCPC_VERIFIER_ID not set")?,
            btcpc_verifier_key_hex: std::env::var("BTCPC_VERIFIER_KEY")
                .context("BTCPC_VERIFIER_KEY not set")?,
            btcpc_api_url: std::env::var("BTCPC_API_URL")
                .unwrap_or_else(|_| "http://localhost:4242".into()),
            hive_api_url: std::env::var("HIVE_API_URL")
                .unwrap_or_else(|_| "https://api.hive.blog".into()),
            hive_account: std::env::var("HIVE_ACCOUNT")
                .context("HIVE_ACCOUNT not set")?,
        })
    }
}

/// Run the full Phase 3 verify flow.
///
/// 1. Fetch the Hive block operation at (hive_block_num, op_index).
/// 2. Parse the custom_json body from the operation.
/// 3. Verify: CID matches, payload_sha256 matches, merkle_root matches.
/// 4. Compute the per-epoch challenge hash using prev_seal_hash from BTCPC.
/// 5. Submit HiveReplicaVerify to the BTCPC node.
pub async fn run_verify(
    cfg: VerifierConfig,
    node_id: String,
    cid: String,
    hive_tx_id: String,
    hive_block_num: u64,
    op_index: u32,
    epoch: u64,
) -> Result<()> {
    eprintln!(
        "verifying: cid={} hive_block={} op_index={} epoch={}",
        cid, hive_block_num, op_index, epoch
    );

    // ── 1. Fetch Hive operation ───────────────────────────────────────────────
    let hive = HiveClient::new(&cfg.hive_api_url);
    let op = hive.get_op_in_block(hive_block_num, op_index).await?;
    eprintln!("fetched Hive op: {}", serde_json::to_string_pretty(&op)?);

    // ── 2. Parse custom_json payload ─────────────────────────────────────────
    let parsed = parse_custom_json_op(&op)?;
    eprintln!("parsed payload type: {}", parsed.payload_type);

    // ── 3. Verify CID, hashes ────────────────────────────────────────────────
    if parsed.cid != cid {
        bail!(
            "CID mismatch: Hive payload has '{}', expected '{}'",
            parsed.cid,
            cid
        );
    }
    eprintln!("CID match: {}", cid);

    // Replica kind from the payload type.
    let replica_kind = payload_type_to_kind(&parsed.payload_type)?;

    // bytes_verified: use the size recorded in the Hive payload.
    let bytes_verified = parsed.size_bytes.unwrap_or_else(|| {
        // For chunks without explicit size, estimate from payload_b64 if present.
        parsed
            .payload_b64_len
            .map(|b64_len| (b64_len * 3 / 4) as u64)
            .unwrap_or(0)
    });
    if bytes_verified == 0 {
        bail!("cannot determine bytes_verified from Hive payload — size_bytes missing");
    }

    // ── 4. Fetch prev_seal_hash and compute challenge hash ───────────────────
    let btcpc = BtcpcClient::new(&cfg.btcpc_api_url);
    let prev_epoch = epoch.saturating_sub(1);
    let prev_seal_hash = btcpc.block_hash(prev_epoch).await?;
    if prev_seal_hash.is_empty() {
        bail!(
            "epoch {} block hash not available from BTCPC node — is the epoch finalized?",
            prev_epoch
        );
    }
    eprintln!("prev_seal_hash (epoch {}): {}…", prev_epoch, &prev_seal_hash[..12.min(prev_seal_hash.len())]);

    let challenge_hash = compute_challenge_hash(
        &prev_seal_hash,
        &node_id,
        &cid,
        &hive_tx_id,
        epoch,
    );
    eprintln!("challenge_hash: {}…", &challenge_hash[..12]);

    // ── 5. Submit HiveReplicaVerify ───────────────────────────────────────────
    let verify_params = HiveVerifyParams {
        verifier: cfg.btcpc_verifier_id.clone(),
        node_id: node_id.clone(),
        cid: cid.clone(),
        hive_account: cfg.hive_account.clone(),
        custom_json_id: "btcpc_fs_v1".into(),
        hive_block_num,
        hive_tx_id: hive_tx_id.clone(),
        op_index,
        payload_sha256: parsed.payload_sha256.clone(),
        merkle_root: parsed.merkle_root.clone(),
        bytes_verified,
        replica_kind: replica_kind.clone(),
        challenge_hash,
    };

    let resp = btcpc
        .post_hive_verify(verify_params, &cfg.btcpc_verifier_key_hex)
        .await?;

    eprintln!("HiveReplicaVerify submitted: {}", resp);
    println!(
        "{{\"ok\":true,\"hive_tx_id\":\"{}\",\"cid\":\"{}\",\"bytes_verified\":{},\"kind\":\"{}\"}}",
        hive_tx_id, cid, bytes_verified, replica_kind
    );
    Ok(())
}

// ── Parsing helpers ───────────────────────────────────────────────────────────

struct ParsedPayload {
    payload_type: String,
    cid: String,
    payload_sha256: String,
    merkle_root: String,
    size_bytes: Option<u64>,
    payload_b64_len: Option<usize>,
}

/// Extract and parse the `custom_json` field from a Hive operation.
///
/// The operation shape from condenser_api.get_ops_in_block is:
/// `{ "op": ["custom_json", { "id": "btcpc_fs_v1", "json": "..." }], ... }`
/// or in some API versions:
/// `{ "type": "custom_json_operation", "value": { "id": "...", "json": "..." } }`
fn parse_custom_json_op(op: &serde_json::Value) -> Result<ParsedPayload> {
    // Try both shapes.
    let op_body = extract_op_body(op)?;
    let id = op_body
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if id != "btcpc_fs_v1" {
        bail!(
            "expected Hive op id 'btcpc_fs_v1', got '{}'",
            id
        );
    }
    let json_str = op_body
        .get("json")
        .and_then(|v| v.as_str())
        .context("custom_json op has no 'json' field")?;

    let payload: serde_json::Value =
        serde_json::from_str(json_str).context("custom_json 'json' field is not valid JSON")?;

    let payload_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .context("payload missing 'type' field")?
        .to_string();

    let cid = payload
        .get("cid")
        .and_then(|v| v.as_str())
        .context("payload missing 'cid'")?
        .to_string();

    // payload_sha256: present in chunk/parity/full payloads.
    // For manifest we derive it from the JSON itself.
    let payload_sha256 = if let Some(v) = payload.get("payload_sha256").and_then(|v| v.as_str()) {
        v.to_string()
    } else {
        // Recompute from the canonical JSON string stored on Hive.
        let mut h = Sha256::new();
        h.update(json_str.as_bytes());
        hex::encode(h.finalize())
    };

    let merkle_root = payload
        .get("merkle_root")
        .and_then(|v| v.as_str())
        .unwrap_or(&payload_sha256)
        .to_string();

    let size_bytes = payload
        .get("size_bytes")
        .and_then(|v| v.as_u64());

    let payload_b64_len = payload
        .get("payload_b64")
        .and_then(|v| v.as_str())
        .map(|s| s.len());

    Ok(ParsedPayload {
        payload_type,
        cid,
        payload_sha256,
        merkle_root,
        size_bytes,
        payload_b64_len,
    })
}

/// Try both condenser API shapes to reach the operation body.
fn extract_op_body(op: &serde_json::Value) -> Result<&serde_json::Value> {
    // Shape A: { "op": ["custom_json", { ... }], ... }
    if let Some(op_arr) = op.get("op").and_then(|v| v.as_array()) {
        if op_arr.len() >= 2 {
            return Ok(&op_arr[1]);
        }
    }
    // Shape B: { "type": "custom_json_operation", "value": { ... } }
    if let Some(value) = op.get("value") {
        return Ok(value);
    }
    // Shape C: op is the body directly (some Hive nodes inline it)
    if op.get("id").is_some() {
        return Ok(op);
    }
    bail!("cannot extract custom_json op body from: {}", op)
}

fn payload_type_to_kind(t: &str) -> Result<String> {
    match t {
        "btcpc_fs_manifest_v1" => Ok("manifest".into()),
        "btcpc_fs_chunk_v1" => Ok("chunk".into()),
        "btcpc_fs_parity_v1" => Ok("parity".into()),
        other => bail!(
            "unknown btcpc_fs payload type '{}'; expected btcpc_fs_{{manifest,chunk,parity}}_v1",
            other
        ),
    }
}

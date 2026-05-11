//! Minimal BTCPC node API client for the hivefs adapter.
//!
//! Handles:
//! - POST /api/storage/hive-replica/commit
//! - POST /api/storage/hive-replica/verify
//! - GET  /api/block/:epoch  (for prev_seal_hash)
//! - GET  /api/latest        (for current epoch)

use anyhow::{Context, Result};
use ed25519_dalek::Signer;
use sha2::{Digest, Sha256};

pub struct BtcpcClient {
    http: reqwest::Client,
    pub api_url: String,
}

impl BtcpcClient {
    pub fn new(api_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_url: api_url.into().trim_end_matches('/').to_owned(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.api_url, path)
    }

    // ── Chain queries ─────────────────────────────────────────────────────────

    /// Current epoch number from the node.
    #[allow(dead_code)]
    pub async fn current_epoch(&self) -> Result<u64> {
        let resp: serde_json::Value = self
            .http
            .get(self.url("/api/latest"))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        resp.get("current_epoch")
            .and_then(|v| v.as_u64())
            .context("missing current_epoch in /api/latest")
    }

    /// Fetch the block hash for `epoch` — used as `prev_seal_hash` for epoch-1.
    ///
    /// Returns the hex hash string, or an empty string if the block is not yet
    /// finalized (verifier should retry later).
    pub async fn block_hash(&self, epoch: u64) -> Result<String> {
        let resp: serde_json::Value = self
            .http
            .get(self.url(&format!("/api/block/{}", epoch)))
            .send()
            .await?
            .json()
            .await?;
        Ok(resp
            .get("hash")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string())
    }

    /// Account nonce (used to sequence signed entries).
    pub async fn get_nonce(&self, account: &str) -> Result<u64> {
        let resp: serde_json::Value = self
            .http
            .get(self.url(&format!("/api/account/{}", account)))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp.get("nonce").and_then(|v| v.as_u64()).unwrap_or(0))
    }

    // ── Hive replica endpoints ────────────────────────────────────────────────

    /// Submit a HiveReplicaCommit entry to the BTCPC node.
    pub async fn post_hive_commit(
        &self,
        params: HiveCommitParams,
        signing_key_hex: &str,
    ) -> Result<serde_json::Value> {
        let nonce = self.get_nonce(&params.node_id).await? + 1;

        let mut body = serde_json::json!({
            "node_id": params.node_id,
            "cid": params.cid,
            "hive_account": params.hive_account,
            "custom_json_id": params.custom_json_id,
            "hive_block_num": params.hive_block_num,
            "hive_tx_id": params.hive_tx_id,
            "op_index": params.op_index,
            "payload_sha256": params.payload_sha256,
            "merkle_root": params.merkle_root,
            "bytes_replicated": params.bytes_replicated,
            "replica_kind": params.replica_kind,
            "confirmations": params.confirmations,
            "nonce": nonce,
        });

        let sig = sign_entry_json(&body, signing_key_hex)?;
        body["signature"] = serde_json::Value::String(sig);

        let resp: serde_json::Value = self
            .http
            .post(self.url("/api/storage/hive-replica/commit"))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        Ok(resp)
    }

    /// Submit a HiveReplicaVerify entry to the BTCPC node.
    pub async fn post_hive_verify(
        &self,
        params: HiveVerifyParams,
        signing_key_hex: &str,
    ) -> Result<serde_json::Value> {
        let nonce = self.get_nonce(&params.verifier).await? + 1;

        let mut body = serde_json::json!({
            "verifier": params.verifier,
            "node_id": params.node_id,
            "cid": params.cid,
            "hive_account": params.hive_account,
            "custom_json_id": params.custom_json_id,
            "hive_block_num": params.hive_block_num,
            "hive_tx_id": params.hive_tx_id,
            "op_index": params.op_index,
            "payload_sha256": params.payload_sha256,
            "merkle_root": params.merkle_root,
            "bytes_verified": params.bytes_verified,
            "replica_kind": params.replica_kind,
            "challenge_hash": params.challenge_hash,
            "nonce": nonce,
        });

        let sig = sign_entry_json(&body, signing_key_hex)?;
        body["signature"] = serde_json::Value::String(sig);

        let resp: serde_json::Value = self
            .http
            .post(self.url("/api/storage/hive-replica/verify"))
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        Ok(resp)
    }
}

// ── Parameter structs ─────────────────────────────────────────────────────────

pub struct HiveCommitParams {
    pub node_id: String,
    pub cid: String,
    pub hive_account: String,
    pub custom_json_id: String,
    pub hive_block_num: u64,
    pub hive_tx_id: String,
    pub op_index: u32,
    pub payload_sha256: String,
    pub merkle_root: String,
    pub bytes_replicated: u64,
    pub replica_kind: String,
    pub confirmations: u32,
}

pub struct HiveVerifyParams {
    pub verifier: String,
    pub node_id: String,
    pub cid: String,
    pub hive_account: String,
    pub custom_json_id: String,
    pub hive_block_num: u64,
    pub hive_tx_id: String,
    pub op_index: u32,
    pub payload_sha256: String,
    pub merkle_root: String,
    pub bytes_verified: u64,
    pub replica_kind: String,
    pub challenge_hash: String,
}

// ── Signing ───────────────────────────────────────────────────────────────────

/// Sign a JSON body with an ed25519 key (hex-encoded 32-byte seed).
///
/// The signature covers sha256(json_canonical_bytes) — matching the
/// btcpc-sdk `KeyPair::sign_entry_json` convention.
pub fn sign_entry_json(body: &serde_json::Value, key_hex: &str) -> Result<String> {
    let key_bytes = hex::decode(key_hex.trim())
        .context("signing key must be 64-char hex")?;
    let key_arr: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("signing key must be exactly 32 bytes"))?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&key_arr);
    let body_bytes = serde_json::to_vec(body)?;
    let sig = signing_key.sign(&body_bytes);
    Ok(hex::encode(sig.to_bytes()))
}

/// Compute the per-epoch challenge hash:
/// sha256("{prev_seal_hash}:{node_id}:hive:{cid}:{hive_tx_id}:{epoch}")
pub fn compute_challenge_hash(
    prev_seal_hash: &str,
    node_id: &str,
    cid: &str,
    hive_tx_id: &str,
    epoch: u64,
) -> String {
    let preimage = format!(
        "{}:{}:hive:{}:{}:{}",
        prev_seal_hash, node_id, cid, hive_tx_id, epoch
    );
    let mut h = Sha256::new();
    h.update(preimage.as_bytes());
    hex::encode(h.finalize())
}

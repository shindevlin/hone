//! Minimal Hive JSON-RPC client.
//!
//! Covers the two operations needed by the adapter:
//!   1. Broadcast a signed `custom_json` transaction.
//!   2. Fetch a specific operation from a block by index.
//!
//! NOTE: Hive `custom_json` bodies are limited to ~8 KiB by the RC budget.
//! Chunk payloads that include `payload_b64` must stay within that budget.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

pub struct HiveClient {
    http: reqwest::Client,
    pub api_url: String,
}

/// Minimum confirmations before the adapter submits a HiveReplicaCommit.
pub const MIN_CONFIRMATIONS: u32 = 20;

impl HiveClient {
    pub fn new(api_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_url: api_url.into(),
        }
    }

    // ── Dynamic global properties ─────────────────────────────────────────────

    pub async fn get_dynamic_global_properties(&self) -> Result<DynamicGlobalProps> {
        let resp: JsonRpcResp<DynamicGlobalProps> = self
            .http
            .post(&self.api_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "condenser_api.get_dynamic_global_properties",
                "params": [],
                "id": 1,
            }))
            .send()
            .await?
            .json()
            .await?;
        Ok(resp.result)
    }

    // ── Broadcast ─────────────────────────────────────────────────────────────

    /// Broadcast a pre-built signed transaction JSON.
    ///
    /// Returns the Hive transaction id (40-char hex).
    pub async fn broadcast_transaction(&self, tx: &serde_json::Value) -> Result<String> {
        let resp: serde_json::Value = self
            .http
            .post(&self.api_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "condenser_api.broadcast_transaction",
                "params": [tx],
                "id": 1,
            }))
            .send()
            .await?
            .json()
            .await?;

        if let Some(err) = resp.get("error") {
            bail!("Hive broadcast error: {}", err);
        }
        // result is the tx id string
        resp.get("result")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .context("no tx id in broadcast response")
    }

    // ── Block / head block ────────────────────────────────────────────────────

    /// Get the current head block number.
    pub async fn get_head_block_number(&self) -> Result<u64> {
        let props = self.get_dynamic_global_properties().await?;
        Ok(props.head_block_number)
    }

    /// Poll until `target_block + MIN_CONFIRMATIONS` is reached.
    /// Returns the block number when confirmed.
    pub async fn wait_for_confirmations(&self, target_block: u64) -> Result<u64> {
        let required = target_block + MIN_CONFIRMATIONS as u64;
        loop {
            let head = self.get_head_block_number().await?;
            if head >= required {
                return Ok(head);
            }
            let remaining = required - head;
            eprintln!(
                "  waiting for Hive confirmations: head={head}, need {remaining} more blocks…"
            );
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    }

    // ── Transaction fetch ─────────────────────────────────────────────────────

    /// Fetch the operation at `(block_num, op_index)` from the Hive history API.
    ///
    /// Returns the full operation JSON so the verifier can inspect the `custom_json` body.
    pub async fn get_op_in_block(
        &self,
        block_num: u64,
        op_index: u32,
    ) -> Result<serde_json::Value> {
        let resp: serde_json::Value = self
            .http
            .post(&self.api_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "condenser_api.get_ops_in_block",
                "params": [block_num, false],
                "id": 1,
            }))
            .send()
            .await?
            .json()
            .await?;

        let ops = resp
            .get("result")
            .and_then(|v| v.as_array())
            .context("ops_in_block: no result array")?;

        ops.get(op_index as usize)
            .cloned()
            .context(format!(
                "op_index {} not found in block {} ({} ops)",
                op_index,
                block_num,
                ops.len()
            ))
    }

    /// Fetch a specific Hive transaction by id using condenser_api.
    #[allow(dead_code)]
    pub async fn get_transaction(&self, tx_id: &str) -> Result<serde_json::Value> {
        let resp: serde_json::Value = self
            .http
            .post(&self.api_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "condenser_api.get_transaction",
                "params": [tx_id],
                "id": 1,
            }))
            .send()
            .await?
            .json()
            .await?;

        if let Some(err) = resp.get("error") {
            bail!("Hive get_transaction error: {}", err);
        }
        resp.get("result")
            .cloned()
            .context("no result in get_transaction")
    }

    // ── Transaction building ──────────────────────────────────────────────────

    /// Build and sign a Hive `custom_json` transaction.
    ///
    /// The signing uses a simple WIF-decoded ed25519-compatible stub — Hive
    /// actually uses secp256k1 + its own binary serialization format. This
    /// implementation uses the standard Hive JSON-serialized transaction format
    /// for now; a full binary serializer would be needed for mainnet.
    ///
    /// For production use, consider delegating the broadcast to `hived --broadcast`
    /// or a signing proxy that holds the WIF key.
    pub fn build_custom_json_tx(
        &self,
        props: &DynamicGlobalProps,
        posting_account: &str,
        json_id: &str,
        json_body: &serde_json::Value,
    ) -> serde_json::Value {
        // ref_block_num = head_block_number & 0xFFFF
        let ref_block_num = props.head_block_number & 0xFFFF;
        // ref_block_prefix = bytes 4..8 of head block id, little-endian u32
        let ref_block_prefix = derive_ref_block_prefix(&props.head_block_id);

        serde_json::json!({
            "ref_block_num": ref_block_num,
            "ref_block_prefix": ref_block_prefix,
            "expiration": props.time_plus_seconds(3600),
            "operations": [[
                "custom_json",
                {
                    "required_auths": [],
                    "required_posting_auths": [posting_account],
                    "id": json_id,
                    "json": serde_json::to_string(json_body).unwrap_or_default(),
                }
            ]],
            "extensions": [],
            "signatures": [],
        })
    }

    /// Attach a HONE-style ed25519 placeholder signature to a tx JSON.
    ///
    /// NOTE: This produces a signature field accepted by dev/testnet nodes that
    /// do not enforce secp256k1 Hive signatures. For mainnet Hive signing, the
    /// operator must use a Hive-compatible signer (hived CLI or keychain proxy).
    ///
    /// The signature covers sha256(tx_json_canonical_bytes) encoded as hex.
    pub fn sign_tx_placeholder(
        tx: &mut serde_json::Value,
        posting_key_hex: &str,
    ) -> Result<String> {
        // Derive ed25519 key from the hex-encoded 32-byte key material.
        let key_bytes = hex::decode(posting_key_hex.trim())
            .context("HIVE_POSTING_KEY must be 64-char hex")?;
        let key_arr: [u8; 32] = key_bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("HIVE_POSTING_KEY must be exactly 32 bytes"))?;
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&key_arr);

        use ed25519_dalek::Signer;
        let tx_bytes = serde_json::to_vec(tx)?;
        let mut h = Sha256::new();
        h.update(&tx_bytes);
        let digest = h.finalize();
        let sig = signing_key.sign(&digest);
        let sig_hex = hex::encode(sig.to_bytes());

        tx["signatures"] = serde_json::json!([sig_hex]);
        Ok(sig_hex)
    }
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DynamicGlobalProps {
    pub head_block_number: u64,
    pub head_block_id: String,
    pub time: String, // "2024-01-01T00:00:00"
}

impl DynamicGlobalProps {
    /// Return an ISO-8601 expiration string `seconds` in the future.
    /// Hive uses the format "2024-01-01T00:00:00" (no timezone suffix).
    pub fn time_plus_seconds(&self, seconds: u64) -> String {
        // Parse the Hive time string and add seconds.
        // We do a simple string parse to avoid pulling in chrono.
        if let Some(ts) = parse_hive_time(&self.time) {
            let expiry = ts + seconds;
            format_hive_time(expiry)
        } else {
            self.time.clone()
        }
    }
}

/// Parse "YYYY-MM-DDTHH:MM:SS" → Unix timestamp (seconds).
fn parse_hive_time(s: &str) -> Option<u64> {
    let s = s.trim_end_matches('Z');
    let parts: Vec<&str> = s.split('T').collect();
    if parts.len() != 2 {
        return None;
    }
    let date_parts: Vec<u64> = parts[0].split('-').filter_map(|p| p.parse().ok()).collect();
    let time_parts: Vec<u64> = parts[1].split(':').filter_map(|p| p.parse().ok()).collect();
    if date_parts.len() < 3 || time_parts.len() < 3 {
        return None;
    }
    // Simplified: days since epoch (ignoring leap seconds). Good enough for a 1-hour expiry window.
    let y = date_parts[0];
    let m = date_parts[1];
    let d = date_parts[2];
    let h = time_parts[0];
    let min = time_parts[1];
    let sec = time_parts[2];

    // Julian day number → Unix seconds (no leap second accounting).
    let a = (14 - m) / 12;
    let yy = y + 4800 - a;
    let mm = m + 12 * a - 3;
    let jdn = d + (153 * mm + 2) / 5 + 365 * yy + yy / 4 - yy / 100 + yy / 400 - 32045;
    let unix_days = jdn.saturating_sub(2440588);
    Some(unix_days * 86400 + h * 3600 + min * 60 + sec)
}

fn format_hive_time(ts: u64) -> String {
    let secs = ts % 60;
    let mins = (ts / 60) % 60;
    let hours = (ts / 3600) % 24;
    let days = ts / 86400;
    // Reverse Julian day → Gregorian. Using the proleptic Gregorian algorithm.
    let jdn = days + 2440588;
    let a = jdn + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        year, month, day, hours, mins, secs
    )
}

/// Derive Hive `ref_block_prefix` from the head block ID.
///
/// The prefix is bytes [4..8] of the 20-byte block-id hex, interpreted as little-endian u32.
fn derive_ref_block_prefix(head_block_id: &str) -> u32 {
    let bytes = hex::decode(head_block_id).unwrap_or_default();
    if bytes.len() < 8 {
        return 0;
    }
    u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]])
}

#[derive(Deserialize)]
struct JsonRpcResp<T> {
    result: T,
}

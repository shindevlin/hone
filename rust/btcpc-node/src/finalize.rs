//! Epoch finalization — runs every `interval_epochs` epochs, creates a
//! finality snapshot and persists it via `store.write_finality`.

use anyhow::Result;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

use btcpc_types::Block;
use crate::chain::Chain;

// ── Public entry point ────────────────────────────────────────────────────────

/// Spawn a long-running finalizer task. Watches `chain.current_epoch` and
/// finalizes every `interval_epochs` epochs.
///
/// # Example
/// ```no_run
/// tokio::spawn(run_finalizer(chain.clone(), 100));
/// ```
pub async fn run_finalizer(chain: Arc<Chain>, interval_epochs: u64) {
    info!(
        "[finalize] finalizer started (interval: {} epochs)",
        interval_epochs
    );

    let mut last_finalized: u64 = 0;

    loop {
        let current = chain.current_epoch();

        // Determine the most recent finalizable boundary.
        let boundary = (current / interval_epochs) * interval_epochs;

        if boundary > 0 && boundary > last_finalized {
            match finalize_epoch(&chain, boundary).await {
                Ok(()) => {
                    last_finalized = boundary;
                }
                Err(e) => {
                    warn!("[finalize] epoch {} finalization failed: {}", boundary, e);
                }
            }
        }

        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }
}

// ── Core finalization ─────────────────────────────────────────────────────────

/// Create and persist a finality snapshot for `epoch`.
pub async fn finalize_epoch(chain: &Arc<Chain>, epoch: u64) -> Result<()> {
    let finalized_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Retrieve the latest block to extract its hash.
    let latest_block_hash = latest_block_hash(chain, epoch);

    // Proxy state root: SHA-256 of (latest_block_hash || epoch_bytes).
    // A full Merkle root over all balances can replace this once scan_all
    // is available.
    let state_root = compute_state_root(&latest_block_hash, epoch);

    let snapshot = serde_json::json!({
        "epoch": epoch,
        "state_root": state_root,
        "finalized_at": finalized_at,
        "latest_block_hash": latest_block_hash,
    });

    let bytes = serde_json::to_vec(&snapshot)?;

    // write_finality takes u32 — epochs are bounded well within u32 range.
    chain.store.write_finality(epoch as u32, &bytes)?;

    info!(
        "[finalize] epoch {} finalized: state_root={} block={}",
        epoch,
        &state_root[..12],
        &latest_block_hash[..12.min(latest_block_hash.len())],
    );

    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Fetch the raw block for `epoch` (or the nearest earlier one) and return
/// its header hash.  Falls back to a deterministic placeholder if no block
/// is found so finalization never blocks on missing data.
fn latest_block_hash(chain: &Arc<Chain>, epoch: u64) -> String {
    // Walk backwards up to 10 epochs to find a stored block.
    let start = epoch as u32;
    for e in (start.saturating_sub(10)..=start).rev() {
        if let Ok(Some(bytes)) = chain.store.read_block(e) {
            // Block format: 180-byte binary header + 4-byte len + JSON payload.
            // Must use Block::from_bytes — serde_json::from_slice fails on the binary header.
            if let Some(block) = Block::from_bytes(&bytes) {
                return block.header.hash_hex();
            }
        }
    }

    // Deterministic fallback: hex of SHA-256("genesis" || epoch).
    let mut hasher = Sha256::new();
    hasher.update(b"genesis");
    hasher.update(epoch.to_le_bytes());
    hex::encode(hasher.finalize())
}

/// Proxy state root: SHA-256( latest_block_hash_bytes || epoch_le_bytes ).
fn compute_state_root(latest_block_hash: &str, epoch: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(latest_block_hash.as_bytes());
    hasher.update(epoch.to_le_bytes());
    hex::encode(hasher.finalize())
}

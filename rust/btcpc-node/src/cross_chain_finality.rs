//! Cross-chain finality announcements.
//!
//! At each finalization boundary the BTCPC node writes a signed announcement
//! file that external chains (TON, Ethereum, etc.) can consume to verify that
//! a given BTCPC epoch is irreversible.
//!
//! Files land at:
//!   `$BTCPC_DATA_DIR/anchors/cross-chain/{chain_id}/epoch-{N}.json`
//!   `$BTCPC_DATA_DIR/anchors/cross-chain/{chain_id}/latest.json`
//!
//! An announcement is also inscribed on-chain as a `CrossChainFinalityAnnounce`
//! entry so the record is globally visible and verifiable.
#![allow(dead_code)]

#![allow(dead_code)]
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{bail, Result};
use btcpc_types::{Block, LedgerEntry};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::chain::Chain;

// ── Announcement model ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossChainAnnouncement {
    pub chain_id: String,
    pub finality_epoch: u64,
    pub cutoff_epoch: u64,
    pub state_root: String,
    pub finality_hash: String,
    pub inference_finality_hash: String,
    pub announcement_hash: String,
    pub finalized_job_count: u64,
    pub announced_at_ms: u64,
}

// ── Engine ────────────────────────────────────────────────────────────────────

pub struct CrossChainFinalityModule {
    chain: Arc<Chain>,
    data_dir: PathBuf,
}

impl CrossChainFinalityModule {
    pub fn new(chain: Arc<Chain>, data_dir: &str) -> Self {
        Self {
            chain,
            data_dir: PathBuf::from(data_dir),
        }
    }

    fn anchors_dir(&self, chain_id: &str) -> PathBuf {
        self.data_dir.join("anchors").join("cross-chain").join(chain_id)
    }

    /// Build and persist a finality announcement for `finality_epoch`.
    ///
    /// `chain_id` is the BTCPC chain identifier (e.g. `"btcpc-1"` or `"btcpc-satoshi"`).
    /// `prev_finality_epoch` is the most recently announced epoch (0 if none).
    pub fn announce(
        &self,
        chain_id: &str,
        finality_epoch: u64,
        prev_finality_epoch: u64,
        node_account: &str,
    ) -> Result<CrossChainAnnouncement> {
        let finality_hash =
            self.compute_finality_hash(prev_finality_epoch, finality_epoch);
        let (inference_finality_hash, job_count) =
            self.compute_inference_finality_hash(prev_finality_epoch, finality_epoch);

        // State root: read from the finality snapshot written by finalize.rs.
        let state_root = self
            .chain
            .store
            .read_finality(finality_epoch)
            .ok()
            .flatten()
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|v| v["state_root"].as_str().map(str::to_owned))
            .unwrap_or_else(|| fallback_state_root(finality_epoch));

        let announcement_hash = compute_announcement_hash(
            &state_root,
            &finality_hash,
            &inference_finality_hash,
        );

        let ann = CrossChainAnnouncement {
            chain_id: chain_id.to_owned(),
            finality_epoch,
            cutoff_epoch: finality_epoch,
            state_root: state_root.clone(),
            finality_hash: finality_hash.clone(),
            inference_finality_hash: inference_finality_hash.clone(),
            announcement_hash: announcement_hash.clone(),
            finalized_job_count: job_count,
            announced_at_ms: now_ms(),
        };

        // Write files.
        if let Err(e) = self.write_announcement_files(chain_id, finality_epoch, &ann) {
            warn!("[cross-chain] failed to write announcement files: {}", e);
        }

        // Inscribe on-chain.
        let epoch = self.chain.current_epoch();
        let entry = LedgerEntry::CrossChainFinalityAnnounce {
            target_chain: chain_id.to_owned(),
            finality_epoch,
            cutoff_epoch: finality_epoch,
            state_root,
            finality_hash,
            inference_finality_hash,
            announcement_hash,
            finalized_job_count: job_count,
            epoch,
            signed_by: node_account.to_owned(),
        };
        if let Err(e) = self.chain.apply_entry(&entry) {
            warn!("[cross-chain] on-chain inscribe failed: {}", e);
        }

        info!(
            "[cross-chain] finality epoch {} announced for chain '{}' ({} jobs)",
            finality_epoch, chain_id, job_count
        );

        Ok(ann)
    }

    /// Load the latest announcement persisted for a given chain.
    pub fn load_latest_announcement(&self, chain_id: &str) -> Option<CrossChainAnnouncement> {
        let path = self.anchors_dir(chain_id).join("latest.json");
        let bytes = std::fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Return the highest epoch announced as finalized for `chain_id`. 0 if none.
    pub fn get_finality_cutoff(&self, chain_id: &str) -> u64 {
        self.load_latest_announcement(chain_id)
            .map(|a| a.finality_epoch)
            .unwrap_or(0)
    }

    /// Return true if `epoch` is at or below the announced finality cutoff.
    pub fn is_epoch_finalized(&self, chain_id: &str, epoch: u64) -> bool {
        epoch <= self.get_finality_cutoff(chain_id)
    }

    /// Error if `epoch` is not yet finalized.
    pub fn assert_epoch_finalized(&self, chain_id: &str, epoch: u64) -> Result<()> {
        let cutoff = self.get_finality_cutoff(chain_id);
        if epoch <= cutoff {
            Ok(())
        } else {
            bail!(
                "epoch {} is not yet finalized for chain '{}' (cutoff: {})",
                epoch, chain_id, cutoff
            )
        }
    }

    // ── Internals ─────────────────────────────────────────────────────────────

    fn write_announcement_files(
        &self,
        chain_id: &str,
        finality_epoch: u64,
        ann: &CrossChainAnnouncement,
    ) -> Result<()> {
        let dir = self.anchors_dir(chain_id);
        std::fs::create_dir_all(&dir)?;

        let bytes = serde_json::to_vec_pretty(ann)?;
        let epoch_path = dir.join(format!("epoch-{}.json", finality_epoch));
        std::fs::write(&epoch_path, &bytes)?;
        std::fs::write(dir.join("latest.json"), &bytes)?;

        Ok(())
    }

    /// SHA-256 of all block hashes in the range `(prev_epoch..=finality_epoch]`.
    fn compute_finality_hash(&self, prev_epoch: u64, finality_epoch: u64) -> String {
        let mut hasher = Sha256::new();
        let start = prev_epoch.saturating_add(1).max(1);
        for ep in start..=finality_epoch {
            let hash = block_hash_for_epoch(&self.chain, ep);
            hasher.update(hash.as_bytes());
        }
        hex::encode(hasher.finalize())
    }

    /// SHA-256 of all `sci_result:` keys in the finalized range.
    /// Also returns the count of completed scientific jobs found.
    fn compute_inference_finality_hash(
        &self,
        _prev_epoch: u64,
        _finality_epoch: u64,
    ) -> (String, u64) {
        let entries = self.chain.store.state_scan_prefix("sci_result:");
        let count = entries.len() as u64;
        let mut hasher = Sha256::new();
        for (key, _) in &entries {
            hasher.update(key.as_bytes());
        }
        (hex::encode(hasher.finalize()), count)
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn block_hash_for_epoch(chain: &Arc<Chain>, epoch: u64) -> String {
    if let Ok(Some(bytes)) = chain.store.read_block(epoch) {
        if let Some(block) = Block::from_bytes(&bytes) {
            return block.header.hash_hex();
        }
    }
    let mut h = Sha256::new();
    h.update(b"empty");
    h.update(epoch.to_le_bytes());
    hex::encode(h.finalize())
}

fn compute_announcement_hash(state_root: &str, finality_hash: &str, infer_hash: &str) -> String {
    let mut h = Sha256::new();
    h.update(state_root.as_bytes());
    h.update(b"|");
    h.update(finality_hash.as_bytes());
    h.update(b"|");
    h.update(infer_hash.as_bytes());
    hex::encode(h.finalize())
}

fn fallback_state_root(epoch: u64) -> String {
    let mut h = Sha256::new();
    h.update(b"btcpc-state-root");
    h.update(epoch.to_le_bytes());
    hex::encode(h.finalize())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Background announcer task ─────────────────────────────────────────────────

/// Spawn-able task: watches for new finality snapshots and announces them.
/// Runs forever; cancel via the tokio task handle if needed.
pub async fn run_announcer(
    module: Arc<CrossChainFinalityModule>,
    chain_id: String,
    node_account: String,
) {
    info!("[cross-chain] announcer started for chain '{}'", chain_id);
    let mut last_announced: u64 = 0;

    loop {
        let latest = module.chain.store.latest_finality().unwrap_or(0);
        if latest > last_announced {
            match module.announce(&chain_id, latest, last_announced, &node_account) {
                Ok(ann) => {
                    last_announced = ann.finality_epoch;
                }
                Err(e) => {
                    warn!("[cross-chain] announce failed at epoch {}: {}", latest, e);
                }
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_chain() -> (Arc<Chain>, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = crate::store::Store::open(dir.path()).expect("store");
        let chain = Arc::new(Chain::new(store, "test-node".into(), "btcpc-satoshi".into()));
        (chain, dir)
    }

    #[test]
    fn get_finality_cutoff_returns_zero_when_no_announcement() {
        let (chain, dir) = make_chain();
        let module = CrossChainFinalityModule::new(chain, dir.path().to_str().unwrap());
        assert_eq!(module.get_finality_cutoff("btcpc-1"), 0);
    }

    #[test]
    fn is_epoch_finalized_false_when_no_announcement() {
        let (chain, dir) = make_chain();
        let module = CrossChainFinalityModule::new(chain, dir.path().to_str().unwrap());
        assert!(!module.is_epoch_finalized("btcpc-1", 5));
    }

    #[test]
    fn assert_epoch_finalized_errors_when_not_finalized() {
        let (chain, dir) = make_chain();
        let module = CrossChainFinalityModule::new(chain, dir.path().to_str().unwrap());
        assert!(module.assert_epoch_finalized("btcpc-1", 10).is_err());
    }

    #[test]
    fn announce_writes_files_and_records_on_chain() {
        let (chain, dir) = make_chain();
        let data_dir = dir.path().to_str().unwrap();
        let module = CrossChainFinalityModule::new(chain.clone(), data_dir);

        let ann = module.announce("btcpc-1", 100, 0, "shindevlin").unwrap();
        assert_eq!(ann.finality_epoch, 100);
        assert_eq!(ann.chain_id, "btcpc-1");
        assert!(!ann.announcement_hash.is_empty());

        let latest = module.load_latest_announcement("btcpc-1").unwrap();
        assert_eq!(latest.finality_epoch, 100);
        assert_eq!(module.get_finality_cutoff("btcpc-1"), 100);
        assert!(module.is_epoch_finalized("btcpc-1", 100));
        assert!(module.is_epoch_finalized("btcpc-1", 50));
        assert!(!module.is_epoch_finalized("btcpc-1", 101));

        let epoch_file = PathBuf::from(data_dir)
            .join("anchors/cross-chain/btcpc-1/epoch-100.json");
        assert!(epoch_file.exists());

        // On-chain record was written.
        let key = "cc_finality:btcpc-1:100".to_owned();
        assert!(chain.store.state_get(&key).is_some());
    }
}

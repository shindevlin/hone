//! Epoch-aligned batch commit of tracker sightings.
//!
//! Privacy model:
//!   commitment_i = SHA256(epoch_window || observer_id || rotating_mac)
//!
//! - epoch_window = epoch / 30  (aligns to ~15-min AirTag MAC rotation at 30s/epoch)
//! - observer_id  = this node's ID — prevents cross-node correlation
//! - rotating_mac = the BLE MAC bytes — never stored on-chain
//!
//! Only counts per tag_type and the SHA256 of the sorted commitment set are
//! submitted on-chain. Raw MACs and individual commitments stay local.

use std::collections::HashMap;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::api::NodeClient;
use crate::classify::TagType;
use crate::scanner::Sighting;

/// Epoch window size for MAC-rotation alignment (in epochs).
const WINDOW_EPOCHS: u64 = 30;

pub struct BatchCommitter {
    observer_id: String,
    account: String,
    epoch_secs: u64,
    client: NodeClient,
    db: sled::Db,
}

impl BatchCommitter {
    pub fn new(
        observer_id: String,
        account: String,
        epoch_secs: u64,
        client: NodeClient,
        db: sled::Db,
    ) -> Self {
        Self { observer_id, account, epoch_secs, client, db }
    }

    /// Run the commit loop — collects sightings each epoch and submits a batch.
    pub async fn run(&self, mut rx: mpsc::Receiver<Sighting>) {
        let mut interval = tokio::time::interval(
            std::time::Duration::from_secs(self.epoch_secs)
        );
        let mut pending: Vec<Sighting> = Vec::new();

        loop {
            tokio::select! {
                Some(s) = rx.recv() => {
                    pending.push(s);
                }
                _ = interval.tick() => {
                    if !pending.is_empty() {
                        self.flush(&pending).await;
                        pending.clear();
                    }
                }
            }
        }
    }

    async fn flush(&self, sightings: &[Sighting]) {
        let epoch = self.current_epoch();
        let window = epoch / WINDOW_EPOCHS;

        // Count by type and collect privacy commitments.
        let mut counts: HashMap<&TagType, u32> = HashMap::new();
        let mut commitments: Vec<String> = Vec::new();

        for s in sightings {
            *counts.entry(&s.tag_type).or_insert(0) += 1;

            let c = self.make_commitment(window, &s.mac);
            commitments.push(c.clone());

            // Persist sighting locally for co-location tracking.
            let key = format!("sighting:{}:{}:{}", epoch, s.tag_type.as_str(), &c[..16]);
            let val = serde_json::json!({
                "mac": s.mac,
                "rssi": s.rssi,
                "ts": s.timestamp,
                "type": s.tag_type.as_str(),
            });
            let _ = self.db.insert(key.as_bytes(), val.to_string().as_bytes());
        }

        commitments.sort();
        let batch_hash = hex::encode(Sha256::digest(commitments.join("|").as_bytes()));

        let body = serde_json::json!({
            "observer_id": self.observer_id,
            "owner": self.account,
            "airtag_count":   counts.get(&TagType::AirTag).copied().unwrap_or(0),
            "android_fmd_count": counts.get(&TagType::AndroidFmd).copied().unwrap_or(0),
            "tile_count":     counts.get(&TagType::Tile).copied().unwrap_or(0),
            "samsung_count":  counts.get(&TagType::SamsungSmartTag).copied().unwrap_or(0),
            "other_count":    counts.get(&TagType::Chipolo).copied().unwrap_or(0)
                            + counts.get(&TagType::Unknown).copied().unwrap_or(0),
            "batch_hash": batch_hash,
            "epoch": epoch,
            "signed_by": self.account,
        });

        info!("epoch {} — committing {} sightings, batch_hash={}", epoch, sightings.len(), &batch_hash[..12]);

        if let Err(e) = self.client.post_tracker_sighting(&body).await {
            warn!("sighting commit failed: {}", e);
        }
    }

    fn make_commitment(&self, window: u64, mac: &str) -> String {
        let mut h = Sha256::new();
        h.update(window.to_le_bytes());
        h.update(self.observer_id.as_bytes());
        h.update(mac.as_bytes());
        hex::encode(h.finalize())
    }

    fn current_epoch(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / self.epoch_secs
    }
}

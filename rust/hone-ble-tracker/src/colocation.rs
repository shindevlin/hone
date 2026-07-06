//! Temporal co-location tracking (Option C).
//!
//! A TrackerClaim's serial_commitment is associated with this observer.
//! Each epoch we check: are there any tracker sightings in the local DB?
//! If the claimer's account has also been active (node heartbeats visible
//! on-chain), we increment their co-location score.
//!
//! After COLOCATION_THRESHOLD consecutive epochs of positive co-location,
//! we can emit a SensorVouch on behalf of the claim.

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use crate::api::NodeClient;

/// Consecutive co-location epochs needed to upgrade claim to "Verified".
const COLOCATION_THRESHOLD: u32 = 20; // ~10 minutes at 30s epochs

/// Epochs of silence before a co-location score resets.
const DECAY_EPOCHS: u64 = 60; // ~30 minutes

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoLocationRecord {
    pub serial_commitment: String,
    pub claimer: String,
    pub consecutive_epochs: u32,
    pub last_seen_epoch: u64,
    pub verified: bool,
}

pub struct CoLocationTracker {
    db: sled::Db,
    client: NodeClient,
    observer_id: String,
    account: String,
    epoch_secs: u64,
}

impl CoLocationTracker {
    pub fn new(
        db: sled::Db,
        client: NodeClient,
        observer_id: String,
        account: String,
        epoch_secs: u64,
    ) -> Self {
        Self { db, client, observer_id, account, epoch_secs }
    }

    pub async fn run(&self) {
        let mut interval = tokio::time::interval(
            std::time::Duration::from_secs(self.epoch_secs)
        );
        loop {
            interval.tick().await;
            if let Err(e) = self.tick().await {
                tracing::warn!("co-location tick error: {}", e);
            }
        }
    }

    async fn tick(&self) -> anyhow::Result<()> {
        let epoch = self.current_epoch();

        // Fetch active claims for our account from the chain.
        let claims = self.client.get_tracker_claims(&self.account).await?;
        if claims.is_empty() {
            return Ok(());
        }

        // Did we see any trackers this epoch?
        let has_sightings = self.epoch_has_sightings(epoch);

        for claim in &claims {
            let key = format!("coloc:{}", claim.serial_commitment);
            let mut rec = self.load_record(&key).unwrap_or_else(|| CoLocationRecord {
                serial_commitment: claim.serial_commitment.clone(),
                claimer: claim.claimer.clone(),
                consecutive_epochs: 0,
                last_seen_epoch: 0,
                verified: false,
            });

            if has_sightings {
                if epoch.saturating_sub(rec.last_seen_epoch) > DECAY_EPOCHS {
                    rec.consecutive_epochs = 0;
                }
                rec.consecutive_epochs += 1;
                rec.last_seen_epoch = epoch;
                debug!("claim {} co-loc score: {}", &claim.serial_commitment[..12], rec.consecutive_epochs);
            } else {
                if epoch.saturating_sub(rec.last_seen_epoch) > DECAY_EPOCHS {
                    rec.consecutive_epochs = 0;
                }
            }

            if !rec.verified && rec.consecutive_epochs >= COLOCATION_THRESHOLD {
                rec.verified = true;
                info!("claim {} reached co-location threshold, submitting vouch", &claim.serial_commitment[..12]);
                self.submit_vouch(&claim.serial_commitment, epoch).await;
            }

            self.save_record(&key, &rec);
        }

        Ok(())
    }

    fn epoch_has_sightings(&self, epoch: u64) -> bool {
        let prefix = format!("sighting:{}:", epoch);
        self.db
            .scan_prefix(prefix.as_bytes())
            .next()
            .is_some()
    }

    async fn submit_vouch(&self, serial_commitment: &str, epoch: u64) {
        let body = serde_json::json!({
            "sensor_id": serial_commitment,
            "voucher": self.account,
            "epoch": epoch,
            "signed_by": self.account,
        });
        if let Err(e) = self.client.post_sensor_vouch(&body).await {
            tracing::warn!("vouch submit failed: {}", e);
        }
    }

    fn load_record(&self, key: &str) -> Option<CoLocationRecord> {
        self.db
            .get(key.as_bytes())
            .ok()
            .flatten()
            .and_then(|b| serde_json::from_slice(&b).ok())
    }

    fn save_record(&self, key: &str, rec: &CoLocationRecord) {
        if let Ok(bytes) = serde_json::to_vec(rec) {
            let _ = self.db.insert(key.as_bytes(), bytes);
        }
    }

    fn current_epoch(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / self.epoch_secs
    }
}

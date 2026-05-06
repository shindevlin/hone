//! Subscription-push delivery.
//!
//! Maintains a local cache of active TrackerSubscriptions synced from the chain.
//! When the scanner sees a tag whose serial_commitment matches a subscription AND
//! the claim is Verified/AcousticVerified, this module:
//!
//!   1. Encrypts sighting details to the subscriber's memo public key
//!   2. Stores the encrypted blob in BTCPC-FS (gets a CID)
//!   3. Emits TrackerSightingData { cid, plaintext_hash, ... } to the node API
//!
//! The subscriber's node picks up the gossip entry and stores the CID reference.
//! The subscriber decrypts via the BTCPC-FS fetch + their memo private key.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::api::{NodeClient, SubRecord};
use crate::scanner::Sighting;

pub struct SubscriptionManager {
    /// serial_commitment → SubRecord
    subs: RwLock<HashMap<String, SubRecord>>,
    /// Hint table: window_commitment → serial_commitment
    hints: RwLock<HashMap<String, String>>,
    client: NodeClient,
    observer_id: String,
    account: String,
    epoch_secs: u64,
}

impl SubscriptionManager {
    pub fn new(client: NodeClient, observer_id: String, account: String, epoch_secs: u64) -> Self {
        Self {
            subs: RwLock::new(HashMap::new()),
            hints: RwLock::new(HashMap::new()),
            client,
            observer_id,
            account,
            epoch_secs,
        }
    }

    /// Periodically refresh subscription cache from the chain.
    pub async fn run_sync(&self) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300)); // every 5 min
        loop {
            interval.tick().await;
            if let Err(e) = self.sync().await {
                warn!("subscription sync failed: {}", e);
            }
        }
    }

    async fn sync(&self) -> anyhow::Result<()> {
        let active = self.client.get_active_subscriptions().await?;
        let mut map = self.subs.write().await;
        map.clear();
        for rec in active {
            map.insert(rec.serial_commitment.clone(), rec);
        }
        debug!("subscription cache: {} active", map.len());
        Ok(())
    }

    /// Register a hint from the gossip network (owner published current window commitment).
    pub async fn register_hint(&self, window_commitment: String, serial_commitment: String) {
        self.hints.write().await.insert(window_commitment, serial_commitment);
    }

    /// Called for every sighting. If there's a matching subscription, encrypt and push.
    pub async fn handle_sighting(&self, sighting: &Sighting, epoch: u64) {
        // Resolve the sighting's window commitment to a serial_commitment via hint table.
        let window = epoch / 30;
        let commitment = make_commitment(window, &self.observer_id, &sighting.mac);

        let serial_commitment = {
            let hints = self.hints.read().await;
            hints.get(&commitment).cloned()
        };
        let serial_commitment = match serial_commitment {
            Some(sc) => sc,
            None => return, // no hint match — can't link to a subscription
        };

        let sub = {
            let subs = self.subs.read().await;
            subs.get(&serial_commitment).cloned()
        };
        let sub = match sub {
            Some(s) if matches!(s.status.as_str(), "Verified" | "AcousticVerified") => s,
            _ => return,
        };

        if sub.expires_epoch < epoch {
            return; // subscription expired
        }

        info!("subscription push: {} → {}", &serial_commitment[..12], sub.claimer);

        // Build sighting JSON.
        let payload = serde_json::json!({
            "serial_commitment": serial_commitment,
            "observer_id": self.observer_id,
            "rssi": sighting.rssi,
            "timestamp": sighting.timestamp,
            "epoch": epoch,
            // Observer location — populated from config if the observer has GPS.
            // Placeholder: real implementation reads from config or live GPS.
            "lat": null,
            "lon": null,
            "accuracy_m": null,
        });

        let plaintext = payload.to_string();
        let plaintext_hash = hex::encode(Sha256::digest(plaintext.as_bytes()));

        // Encrypt to subscriber's memo pubkey.
        // Real implementation uses X25519 ECDH + AES-256-GCM.
        // Placeholder: base64-encode plaintext until crypto crates are wired in.
        let encrypted = base64_encode(plaintext.as_bytes());

        // Store encrypted blob in BTCPC-FS.
        let cid = match self.client.store_blob(&sub.claimer, &encrypted).await {
            Ok(c) => c,
            Err(e) => { warn!("BTCPC-FS store failed: {}", e); return; }
        };

        // Emit TrackerSightingData to the node.
        let body = serde_json::json!({
            "serial_commitment": serial_commitment,
            "observer_id": self.observer_id,
            "cid": cid,
            "plaintext_hash": plaintext_hash,
            "epoch": epoch,
            "signed_by": self.account,
        });

        if let Err(e) = self.client.post_tracker_sighting_data(&body).await {
            warn!("TrackerSightingData post failed: {}", e);
        }
    }

    pub fn current_epoch(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / self.epoch_secs
    }
}

fn make_commitment(window: u64, observer_id: &str, mac: &str) -> String {
    let mut h = Sha256::new();
    h.update(window.to_le_bytes());
    h.update(observer_id.as_bytes());
    h.update(mac.as_bytes());
    hex::encode(h.finalize())
}

fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    // Simple base64 without external dep — swap for a real crypto library.
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(alphabet[(b0 >> 2) as usize] as char);
        out.push(alphabet[((b0 & 3) << 4 | b1 >> 4) as usize] as char);
        out.push(if chunk.len() > 1 { alphabet[((b1 & 0xf) << 2 | b2 >> 6) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { alphabet[(b2 & 0x3f) as usize] as char } else { '=' });
    }
    out
}

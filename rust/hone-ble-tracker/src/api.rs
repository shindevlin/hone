//! HTTP client for the local hone-node API.

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct NodeClient {
    base: String,
    http: reqwest::Client,
}

/// Minimal representation of an active TrackerClaim returned by the node.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaimInfo {
    pub serial_commitment: String,
    pub claimer: String,
    pub tag_type: String,
    pub status: String, // "Registered" | "Verified" | "AcousticVerified"
}

/// Active subscription record returned by /api/tracker/subscriptions/active.
#[derive(Debug, Clone, Deserialize)]
pub struct SubRecord {
    pub serial_commitment: String,
    pub claimer: String,
    /// Hex-encoded memo public key (X25519 or ed25519-derived).
    pub memo_pubkey: String,
    pub expires_epoch: u64,
    pub status: String, // "Verified" | "AcousticVerified"
}

impl NodeClient {
    pub fn new(base: String) -> Self {
        Self {
            base,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
        }
    }

    /// POST /api/tracker/sighting — submit a TrackerSightingCommit entry.
    pub async fn post_tracker_sighting(&self, body: &serde_json::Value) -> anyhow::Result<()> {
        self.post("/api/tracker/sighting", body).await
    }

    /// POST /api/tracker/acoustic-proof — submit a TrackerAcousticProof entry.
    pub async fn post_acoustic_proof(&self, body: &serde_json::Value) -> anyhow::Result<()> {
        self.post("/api/tracker/acoustic-proof", body).await
    }

    /// POST /api/sensor/vouch — submit a SensorVouch for co-location upgrade.
    pub async fn post_sensor_vouch(&self, body: &serde_json::Value) -> anyhow::Result<()> {
        self.post("/api/sensor/vouch", body).await
    }

    /// GET /api/tracker/claims?owner=<account> — fetch active claims for this observer.
    pub async fn get_tracker_claims(&self, owner: &str) -> anyhow::Result<Vec<ClaimInfo>> {
        let url = format!("{}/api/tracker/claims?owner={}", self.base, owner);
        let resp = self.http.get(&url).send().await.context("GET tracker/claims")?;
        if !resp.status().is_success() {
            return Ok(vec![]);
        }
        let claims: Vec<ClaimInfo> = resp.json().await.context("parse claims")?;
        Ok(claims)
    }

    /// GET /api/tracker/subscriptions/active — fetch all active subscriptions.
    pub async fn get_active_subscriptions(&self) -> anyhow::Result<Vec<SubRecord>> {
        let url = format!("{}/api/tracker/subscriptions/active", self.base);
        let resp = self.http.get(&url).send().await.context("GET subscriptions/active")?;
        if !resp.status().is_success() {
            return Ok(vec![]);
        }
        let subs: Vec<SubRecord> = resp.json().await.context("parse subscriptions")?;
        Ok(subs)
    }

    /// POST /api/tracker/sighting-data — emit a TrackerSightingData entry.
    pub async fn post_tracker_sighting_data(&self, body: &serde_json::Value) -> anyhow::Result<()> {
        self.post("/api/tracker/sighting-data", body).await
    }

    /// Store an encrypted blob in HONE-FS, returning its CID.
    /// Placeholder: POSTs to /api/fs/store; returns a synthetic CID until HONE-FS is wired.
    pub async fn store_blob(&self, _owner: &str, data: &str) -> anyhow::Result<String> {
        use sha2::{Digest, Sha256};
        // Stub: derive a deterministic CID from the content hash until HONE-FS is live.
        let hash = hex::encode(Sha256::digest(data.as_bytes()));
        Ok(format!("honefs:{}", &hash[..32]))
    }

    async fn post(&self, path: &str, body: &serde_json::Value) -> anyhow::Result<()> {
        let url = format!("{}{}", self.base, path);
        let resp = self.http
            .post(&url)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {}", path))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("POST {} => {} — {}", path, status, text);
        }
        Ok(())
    }
}

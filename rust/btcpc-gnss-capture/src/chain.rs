use crate::config::Config;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{info, warn};

pub struct ChainRecorder {
    client:     Client,
    config:     Config,
    sensor_id:  String,
    registered: bool,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl ChainRecorder {
    pub fn new(config: Config) -> Self {
        let sensor_id = format!("{}/gnss-base", config.miner);
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(8))
                .build()
                .expect("reqwest client"),
            config,
            sensor_id,
            registered: false,
        }
    }

    async fn ensure_registered(&mut self) {
        if self.registered { return; }

        let url = format!("http://127.0.0.1:{}/api/sensor/register", self.config.api_port);
        let body = serde_json::json!({
            "sensor_id": self.sensor_id,
            "owner": self.config.miner,
            "sensor_type": "gps",
            "location": null,
            "metadata": {
                "protocol": "rtcm3",
                "hardware": "hyfix-mobilecm",
                "device_ip": self.config.device_ip,
            },
            "signature": "",
        });

        match self.client.post(&url).json(&body).send().await {
            Ok(r) if matches!(r.status().as_u16(), 200 | 201 | 409) => {
                info!("GNSS sensor registered: {}", self.sensor_id);
                self.registered = true;
            }
            Ok(r) => warn!("sensor register HTTP {}", r.status()),
            Err(e) => warn!("sensor register: {}", e),
        }
    }

    pub async fn record(&mut self, rtcm_type: u16, payload_bytes: usize) {
        self.ensure_registered().await;
        if !self.registered { return; }

        let ts = now_ms();
        let batch_hash = hex::encode(
            Sha256::digest(format!("{rtcm_type}:{payload_bytes}:{ts}").as_bytes())
        );

        let url = format!("http://127.0.0.1:{}/api/sensor/commit", self.config.api_port);
        let body = serde_json::json!({
            "sensor_id": self.sensor_id,
            "owner": self.config.miner,
            "batch_hash": batch_hash,
            "reading_count": 1u64,
            "sensor_type": "gps",
            "signature": "",
        });

        match self.client.post(&url).json(&body).send().await {
            Ok(r) if r.status().is_success() => {
                info!(rtcm_type, payload_bytes, "GNSS frame committed to chain");
            }
            Ok(r) => {
                let status = r.status();
                let text = r.text().await.unwrap_or_default();
                warn!(rtcm_type, %status, %text, "chain commit rejected");
            }
            Err(e) => warn!("chain commit: {}", e),
        }
    }
}

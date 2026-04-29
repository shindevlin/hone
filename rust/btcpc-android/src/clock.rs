//! Clock consensus participation via btcpc node API.
//! Sends epoch seals every 30 seconds and tracks peer clock state.

use std::sync::Arc;
use std::time::Duration;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use btcpc_types::EPOCH_MS;

pub struct ClockConfig {
    pub account:     String,
    pub api_base:    String,
    pub chain_id:    String,
    pub genesis_ts:  u64,
    pub posting_key: String,
}

pub async fn run(
    cfg: ClockConfig,
    status: Arc<Mutex<String>>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("clock http client");

    let node_id = format!("{}-android", cfg.account);
    let mut last_sent_epoch: u64 = 0;

    *status.lock() = format!("Clock: waiting for genesis…");

    loop {
        tokio::select! {
            _ = &mut shutdown => break,
            _ = tokio::time::sleep(Duration::from_secs(5)) => {}
        }

        let now = now_ms();
        if now < cfg.genesis_ts {
            let wait_secs = (cfg.genesis_ts - now) / 1000;
            *status.lock() = format!("Clock: {}s until genesis", wait_secs);
            continue;
        }

        let elapsed = now - cfg.genesis_ts;
        let epoch = elapsed / EPOCH_MS;

        if epoch <= last_sent_epoch {
            continue;
        }
        last_sent_epoch = epoch;

        let seal_hash = {
            let mut h = Sha256::new();
            h.update(epoch.to_le_bytes());
            h.update(node_id.as_bytes());
            hex::encode(h.finalize())
        };

        let seal = serde_json::json!({
            "epoch_number": epoch,
            "node_id":      node_id,
            "timestamp":    now,
            "seal_hash":    seal_hash,
            "signature":    null,
        });

        let url = format!("{}/api/clock/seal", cfg.api_base);
        match client
            .post(&url)
            .json(&seal)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                *status.lock() = format!("Clock ✓ epoch {} | {}", epoch, cfg.account);
                info!("clock seal sent: epoch {}", epoch);
            }
            Ok(resp) => {
                warn!("clock seal rejected: {}", resp.status());
                *status.lock() = format!("Clock: seal rejected ({})", resp.status());
            }
            Err(e) => {
                warn!("clock seal error: {}", e);
                *status.lock() = format!("Clock: {} — retrying", e);
            }
        }
    }

    *status.lock() = "Clock: stopped".to_owned();
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

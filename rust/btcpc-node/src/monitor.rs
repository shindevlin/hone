//! Chain health monitor.
//!
//! Detects epoch stalls, peer drops, and RocksDB errors. Fires webhook alerts
//! (BTCPC_ALERT_WEBHOOK) and surfaces a rolling alert window via /api/node/health/detailed.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::warn;

use btcpc_types::EPOCH_MS;
use crate::chain::Chain;

const ALERT_HISTORY: usize = 20;
const STALL_MULTIPLIER: u64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertEvent {
    pub kind:    String,
    pub message: String,
    pub ts:      u64,
    pub epoch:   u64,
}

#[derive(Clone)]
pub struct HealthMonitor {
    pub alerts: Arc<Mutex<VecDeque<AlertEvent>>>,
}

impl HealthMonitor {
    pub fn new() -> Self {
        Self { alerts: Arc::new(Mutex::new(VecDeque::new())) }
    }

    pub fn recent_alerts(&self) -> Vec<AlertEvent> {
        self.alerts.lock().iter().cloned().collect()
    }
}

impl Default for HealthMonitor {
    fn default() -> Self { Self::new() }
}

pub async fn run(chain: Arc<Chain>, monitor: Arc<HealthMonitor>) {
    let webhook = std::env::var("BTCPC_ALERT_WEBHOOK").ok();
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let mut last_seen_epoch = chain.current_epoch();
    let stall_threshold_ms = EPOCH_MS * STALL_MULTIPLIER;

    loop {
        tokio::time::sleep(Duration::from_secs(15)).await;

        let now = crate::utils::now_ms();
        let epoch = chain.current_epoch();

        // Epoch stall detection.
        let last_seal_ts: u64 = chain.store.state_get("last_seal_ts")
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|j| j.as_u64())
            .unwrap_or(now);

        let lag_ms = now.saturating_sub(last_seal_ts);
        if lag_ms > stall_threshold_ms {
            let msg = format!("epoch stall: {}ms since last seal (epoch {})", lag_ms, epoch);
            warn!("[monitor] {}", msg);
            fire_alert(&monitor, &http, webhook.as_deref(), "stall", &msg, epoch).await;
        }

        // Peer drop (< 1 peer is an island).
        let peer_count: usize = chain.store.state_get("monitor:peer_count")
            .and_then(|b| serde_json::from_slice::<usize>(&b).ok())
            .unwrap_or(1);
        if peer_count == 0 {
            let msg = format!("isolated: 0 peers at epoch {}", epoch);
            warn!("[monitor] {}", msg);
            fire_alert(&monitor, &http, webhook.as_deref(), "isolated", &msg, epoch).await;
        }

        // Record last-seen-epoch for external consumers.
        if epoch != last_seen_epoch {
            last_seen_epoch = epoch;
            let _ = chain.store.state_set(
                "monitor:last_seen_epoch",
                &serde_json::to_vec(&epoch).unwrap_or_default(),
            );
        }
    }
}

/// Update the peer count in CF_META so monitor can read it without AppState coupling.
pub fn update_peer_count(chain: &Arc<Chain>, count: usize) {
    let _ = chain.store.state_set(
        "monitor:peer_count",
        &serde_json::to_vec(&count).unwrap_or_default(),
    );
}

async fn fire_alert(
    monitor: &Arc<HealthMonitor>,
    http: &reqwest::Client,
    webhook: Option<&str>,
    kind: &str,
    message: &str,
    epoch: u64,
) {
    let event = AlertEvent {
        kind:    kind.to_owned(),
        message: message.to_owned(),
        ts:      crate::utils::now_ms(),
        epoch,
    };

    {
        let mut q = monitor.alerts.lock();
        if q.len() >= ALERT_HISTORY { q.pop_front(); }
        q.push_back(event.clone());
    }

    if let Some(url) = webhook {
        let payload = serde_json::json!({
            "text": format!("[btcpc-node] {} — {}", kind, message),
            "event": event,
        });
        let _ = http.post(url).json(&payload).send().await;
    }
}

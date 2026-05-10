use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};
use btcpc_types::{LedgerEntry, EPOCH_MS};

use crate::chain::Chain;
use crate::net::NetCmd;
use crate::utils::now_ms;

pub async fn run_service_node(
    chain:       Arc<Chain>,
    account:     String,
    genesis_ts:  u64,
    cmd_tx:      tokio::sync::mpsc::Sender<NetCmd>,
) {
    info!("service node started: account={}", account);

    let elapsed = now_ms().saturating_sub(genesis_ts);
    let mut last_epoch = elapsed / EPOCH_MS;

    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;

        let elapsed = now_ms().saturating_sub(genesis_ts);
        let epoch = elapsed / EPOCH_MS;
        if epoch <= last_epoch { continue; }
        last_epoch = epoch;

        let uptime_ms = now_ms().saturating_sub(genesis_ts);
        let container_hours = uptime_ms / 3_600_000;

        let entry = LedgerEntry::ServiceHeartbeat {
            node_id:         account.clone(),
            epoch,
            container_hours,
            signed_by:       account.clone(),
        };

        if let Err(e) = chain.apply_entry(&entry) {
            warn!("service: heartbeat failed epoch {}: {}", epoch, e);
            continue;
        }

        let envelope = serde_json::json!({"entry": entry});
        if let Ok(data) = serde_json::to_vec(&envelope) {
            let _ = cmd_tx.send(NetCmd::Broadcast {
                topic: "btcpc/entries",
                data,
            }).await;
        }

        info!("service: heartbeat epoch {} hours={}", epoch, container_hours);
    }
}

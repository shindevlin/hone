use std::sync::Arc;
use std::time::Duration;
use std::path::Path;
use tracing::{info, warn};
use btcpc_types::{LedgerEntry, EPOCH_MS};

use crate::chain::Chain;
use crate::net::NetCmd;
use crate::utils::now_ms;

pub async fn run_storage_node(
    chain:      Arc<Chain>,
    account:    String,
    data_dir:   std::path::PathBuf,
    genesis_ts: u64,
    cmd_tx:     tokio::sync::mpsc::Sender<NetCmd>,
) {
    info!("storage node started: account={}", account);

    let elapsed = now_ms().saturating_sub(genesis_ts);
    let mut last_epoch = elapsed / EPOCH_MS;

    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;

        let elapsed = now_ms().saturating_sub(genesis_ts);
        let epoch = elapsed / EPOCH_MS;
        if epoch <= last_epoch { continue; }
        last_epoch = epoch;

        let bytes_proven = dir_size_bytes(&data_dir);

        let entry = LedgerEntry::StorageHeartbeat {
            node_id:            account.clone(),
            epoch,
            bytes_proven,
            query_count:        0,
            challenge_response: None,
            signed_by:          account.clone(),
        };

        if let Err(e) = chain.apply_entry(&entry) {
            warn!("storage: heartbeat failed epoch {}: {}", epoch, e);
            continue;
        }

        let envelope = serde_json::json!({"entry": entry});
        if let Ok(data) = serde_json::to_vec(&envelope) {
            let _ = cmd_tx.send(NetCmd::Broadcast {
                topic: "btcpc/entries",
                data,
            }).await;
        }

        info!("storage: heartbeat epoch {} bytes={}", epoch, bytes_proven);
    }
}

fn dir_size_bytes(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    let mut total = 0u64;
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_file() {
            total += meta.len();
        } else if meta.is_dir() {
            total += dir_size_bytes(&entry.path());
        }
    }
    total
}

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::path::Path;
use tracing::{info, warn};
use btcpc_types::{LedgerEntry, EPOCH_MS};

use crate::chain::Chain;
use crate::net::NetCmd;
use crate::utils::now_ms;

pub async fn run_storage_node(
    chain:             Arc<Chain>,
    account:           String,
    data_dir:          std::path::PathBuf,
    genesis_ts:        u64,
    cmd_tx:            tokio::sync::mpsc::Sender<NetCmd>,
    git_serve_queries: Arc<std::sync::atomic::AtomicUsize>,
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
        // Consume git serve query count for this epoch.
        let git_queries = git_serve_queries.swap(0, Ordering::Relaxed) as u64;

        // Tier is derived from bytes proven: <10GB=1, 10-100GB=2, ≥100GB=3.
        let bytes_gb = bytes_proven / (1024 * 1024 * 1024);
        let tier: u8 = if bytes_gb >= 100 { 3 } else if bytes_gb >= 10 { 2 } else { 1 };

        let entry = LedgerEntry::StorageHeartbeat {
            node_id:            account.clone(),
            epoch,
            bytes_proven,
            query_count:        git_queries,
            challenge_response: None,
            tier:               Some(tier),
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

        info!("storage: heartbeat epoch {} bytes={} git_queries={}", epoch, bytes_proven, git_queries);
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

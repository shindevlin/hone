use std::sync::Arc;
use std::time::Duration;
use sha2::{Sha256, Digest};
use tracing::{info, warn};
use btcpc_types::{LedgerEntry, EPOCH_MS};

use crate::chain::Chain;
use crate::net::NetCmd;
use crate::utils::now_ms;

pub async fn run_sensor_node(
    chain:      Arc<Chain>,
    account:    String,
    genesis_ts: u64,
    cmd_tx:     tokio::sync::mpsc::Sender<NetCmd>,
) {
    info!("sensor node started: account={}", account);

    // Register sensor on first run.
    let sensor_id = format!("{}/system", account);
    {
        let reg_entry = LedgerEntry::SensorRegister {
            sensor_id:   sensor_id.clone(),
            owner:       account.clone(),
            sensor_type: "sampled".to_owned(),
            location:    None,
            metadata:    Some(serde_json::json!({
                "hardware": "linux-system",
                "sensors": ["cpu_temp_mC", "uptime_s"],
            })),
            epoch:       0,
            signed_by:   account.clone(),
        };
        let _ = chain.apply_entry(&reg_entry);
    }

    let elapsed = now_ms().saturating_sub(genesis_ts);
    let mut last_epoch = elapsed / EPOCH_MS;

    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;

        let elapsed = now_ms().saturating_sub(genesis_ts);
        let epoch = elapsed / EPOCH_MS;
        if epoch <= last_epoch { continue; }
        last_epoch = epoch;

        let cpu_temp_mc = read_cpu_temp_milli_celsius();
        let uptime_s    = read_uptime_secs();

        let batch = serde_json::json!({
            "epoch":      epoch,
            "sensor_id":  sensor_id,
            "cpu_temp_mC": cpu_temp_mc,
            "uptime_s":   uptime_s,
            "ts":         now_ms(),
        });
        let batch_hash = hex::encode(Sha256::digest(batch.to_string().as_bytes()));

        let entry = LedgerEntry::SensorDataCommit {
            sensor_id:     sensor_id.clone(),
            owner:         account.clone(),
            batch_hash:    batch_hash.clone(),
            reading_count: 1,
            sensor_type:   "sampled".to_owned(),
            epoch,
            signed_by:     account.clone(),
        };

        if let Err(e) = chain.apply_entry(&entry) {
            warn!("sensor: commit failed epoch {}: {}", epoch, e);
            continue;
        }

        let envelope = serde_json::json!({"entry": entry});
        if let Ok(data) = serde_json::to_vec(&envelope) {
            let _ = cmd_tx.send(NetCmd::Broadcast {
                topic: "btcpc/entries",
                data,
            }).await;
        }

        info!("sensor: commit epoch {} cpu_temp={}mC uptime={}s hash={}", epoch, cpu_temp_mc, uptime_s, &batch_hash[..12]);
    }
}

fn read_cpu_temp_milli_celsius() -> i64 {
    std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
        .ok()
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0)
}

fn read_uptime_secs() -> u64 {
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .map(|f| f as u64)
        .unwrap_or(0)
}

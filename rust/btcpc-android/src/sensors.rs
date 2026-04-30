//! Sensor reading submission for the Android micronode.
//! Java calls nativeSubmitReading; this module builds the LedgerEntry and
//! applies it to the local chain + broadcasts it over gossipsub.

use std::sync::Arc;
use sha2::{Digest, Sha256};
use tracing::warn;

use btcpc_types::LedgerEntry;
use crate::chain::Chain;
use crate::net::NetCmd;

pub struct SensorReading {
    pub sensor_id: String,
    pub sensor_type: String,
    pub value: f64,
    pub lat: f64,
    pub lon: f64,
    pub owner: String,
    pub epoch: u64,
    pub metadata: String,
}

pub async fn submit(
    reading: SensorReading,
    chain: Arc<Chain>,
    cmd_tx: tokio::sync::mpsc::Sender<NetCmd>,
) {
    // Build a hash of the reading for on-chain commitment.
    let hash_input = format!(
        "{}:{}:{}:{}:{}:{}",
        reading.sensor_id, reading.sensor_type,
        reading.value, reading.lat, reading.lon, reading.epoch
    );
    let data_hash = hex::encode(Sha256::digest(hash_input.as_bytes()));

    let entry = LedgerEntry::SensorReading {
        sensor_id: reading.sensor_id.clone(),
        owner:     reading.owner.clone(),
        epoch:     reading.epoch,
        value:     reading.value,
        data_hash: data_hash.clone(),
        metadata:  Some(serde_json::json!({
            "type":  reading.sensor_type,
            "lat":   reading.lat,
            "lon":   reading.lon,
            "raw":   reading.metadata,
        })),
    };

    if let Err(e) = chain.apply_entry(&entry) {
        warn!("sensors: apply failed: {}", e);
        return;
    }

    // Broadcast to network so the sensor reward can be allocated.
    let envelope = serde_json::json!({ "entry": entry });
    if let Ok(data) = serde_json::to_vec(&envelope) {
        let _ = cmd_tx.send(NetCmd::Broadcast {
            topic: "btcpc/entries",
            data,
        }).await;
    }
}

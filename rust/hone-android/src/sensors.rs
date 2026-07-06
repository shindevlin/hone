//! Sensor reading submission for the Android micronode.
//!
//! Handles every sensor type the Java service can provide:
//! accelerometer, linear-acceleration, gravity, gyroscope, orientation,
//! orientation-geo, light, barometer, magnetometer, proximity,
//! steps, step-detector, heart-rate, GPS, battery, audio-level.
//!
//! Java calls nativeSubmitReading; this builds the LedgerEntry,
//! applies it locally, and broadcasts over hone/entries gossip.

use std::sync::Arc;
use sha2::{Digest, Sha256};
use tracing::warn;

use hone_types::LedgerEntry;
use crate::chain::Chain;
use crate::net::NetCmd;

pub struct SensorReading {
    /// "account/deviceName-sensorType"
    pub sensor_id:    String,
    pub sensor_type:  String,
    /// Primary scalar value (magnitude, lux, hPa, bpm, etc.)
    pub primary_value: f64,
    /// Full multi-axis or composite JSON: {"x":1.2,"y":0.3,"z":9.8}
    /// or {"lat":53.3,"lon":-6.2,"accuracy":5.0} etc.
    pub values_json:   String,
    pub unit:          String,
    pub owner:         String,
    pub epoch:         u64,
}

pub async fn submit(
    reading: SensorReading,
    chain:   Arc<Chain>,
    cmd_tx:  tokio::sync::mpsc::Sender<NetCmd>,
) {
    // Deterministic hash of the reading for on-chain commitment.
    let hash_input = format!(
        "{}:{}:{}:{}",
        reading.sensor_id, reading.sensor_type,
        reading.primary_value.to_bits(), reading.epoch
    );
    let data_hash = hex::encode(Sha256::digest(hash_input.as_bytes()));

    // Parse values JSON for metadata enrichment.
    let values: serde_json::Value = serde_json::from_str(&reading.values_json)
        .unwrap_or(serde_json::Value::Null);

    let metadata = serde_json::json!({
        "type":    reading.sensor_type,
        "unit":    reading.unit,
        "values":  values,
    });

    let entry = LedgerEntry::SensorReading {
        sensor_id: reading.sensor_id.clone(),
        owner:     reading.owner.clone(),
        epoch:     reading.epoch,
        value:     reading.primary_value,
        data_hash: data_hash.clone(),
        metadata:  Some(metadata),
        // Unsigned bootstrap submission — the micronode does not attach a
        // posting-key signature; verification is skipped until the owner
        // registers a key (see entry.rs SensorReading::signed_by docs).
        signed_by: String::new(),
    };

    if let Err(e) = chain.apply_entry(&entry) {
        warn!("sensors: apply failed: {}", e);
        return;
    }

    let envelope = serde_json::json!({ "entry": entry });
    if let Ok(data) = serde_json::to_vec(&envelope) {
        let _ = cmd_tx.send(NetCmd::Broadcast {
            topic: "hone/entries",
            data,
        }).await;
    }
}

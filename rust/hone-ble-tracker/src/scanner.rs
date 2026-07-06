//! Passive BLE scanner — continuously scans for tracker advertisements and
//! sends Sighting events to a channel. Never connects to or pairs with devices.

use anyhow::Context;
use btleplug::api::{Central, CentralEvent, CentralState, Manager as _, Peripheral, ScanFilter};
use btleplug::platform::Manager;
use futures::StreamExt;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::classify::{self, TagType};

/// A single raw tracker sighting from the BLE scanner.
#[derive(Debug, Clone)]
pub struct Sighting {
    /// Rotating BLE MAC (6 bytes, hex). Never stored on-chain.
    pub mac: String,
    pub tag_type: TagType,
    /// Signal strength in dBm.
    pub rssi: i16,
    /// Unix timestamp (seconds).
    pub timestamp: u64,
}

/// Launch the BLE scanner on the default adapter.
/// Sends Sightings on `tx`; runs until the task is cancelled.
pub async fn run(tx: mpsc::Sender<Sighting>, min_rssi: i16) -> anyhow::Result<()> {
    let manager = Manager::new().await.context("BLE manager init")?;
    let adapters = manager.adapters().await.context("BLE adapters list")?;
    let adapter = adapters
        .into_iter()
        .next()
        .context("No BLE adapter found — is bluetoothd running?")?;

    info!("BLE adapter ready, starting passive scan");
    adapter.start_scan(ScanFilter::default()).await.context("start_scan")?;

    let mut events = adapter.events().await.context("BLE event stream")?;

    while let Some(event) = events.next().await {
        match event {
            CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => {
                let peripheral = match adapter.peripheral(&id).await {
                    Ok(p) => p,
                    Err(e) => { debug!("peripheral fetch error: {}", e); continue; }
                };
                let props = match peripheral.properties().await {
                    Ok(Some(p)) => p,
                    _ => continue,
                };

                let rssi = props.rssi.unwrap_or(i16::MIN);
                if rssi < min_rssi {
                    continue;
                }

                let tag_type = classify::classify(&props.manufacturer_data, &props.services);
                if !classify::is_known_tracker(&tag_type) {
                    continue;
                }

                let mac = props.address.to_string();
                let timestamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                debug!("sighting: {} {:?} rssi={}", mac, tag_type, rssi);

                if tx.send(Sighting { mac, tag_type, rssi, timestamp }).await.is_err() {
                    warn!("scanner: receiver dropped, shutting down");
                    break;
                }
            }
            CentralEvent::StateUpdate(CentralState::PoweredOff) => {
                warn!("BLE adapter powered off");
            }
            _ => {}
        }
    }

    Ok(())
}

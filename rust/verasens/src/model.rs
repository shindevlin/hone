//! Core domain types for Verasens aggregation.
//!
//! These types are independent of the chain's wire format (`btcpc_types::LedgerEntry`)
//! on purpose: `ingest.rs` is the only place that knows how to turn a raw
//! `SensorReading` entry into a `ParsedReading`. Everything downstream (the
//! aggregation index, the query API) operates on `ParsedReading` so it can be
//! unit-tested without constructing real chain entries.

use serde::{Deserialize, Serialize};

/// Sensor-class discriminator, per the Phase 1.2 ratified `metadata` JSON
/// convention (docs/PLATFORM_PRD.md). New classes are added here by
/// convention only — never by a chain change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorClass {
    Gnss,
    Subghz,
    Ir,
    Nfc,
    Rfid,
    Ble,
    HeliumWitness,
}

impl SensorClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            SensorClass::Gnss => "gnss",
            SensorClass::Subghz => "subghz",
            SensorClass::Ir => "ir",
            SensorClass::Nfc => "nfc",
            SensorClass::Rfid => "rfid",
            SensorClass::Ble => "ble",
            SensorClass::HeliumWitness => "helium_witness",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "gnss" => Some(SensorClass::Gnss),
            "subghz" => Some(SensorClass::Subghz),
            "ir" => Some(SensorClass::Ir),
            "nfc" => Some(SensorClass::Nfc),
            "rfid" => Some(SensorClass::Rfid),
            "ble" => Some(SensorClass::Ble),
            "helium_witness" => Some(SensorClass::HeliumWitness),
            _ => None,
        }
    }
}

impl std::fmt::Display for SensorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Geography bucket: a quantized (lat, lon) grid cell.
///
/// Uses the same quantization convention as the chain's `CoverageReport`
/// handling (`chain.rs`: `floor(lat * COVERAGE_GRID_RESOLUTION)`), reusing
/// `COVERAGE_GRID_RESOLUTION = 1000` (~0.001 degree / ~100m cells) as the
/// default grid resolution so Verasens geo-buckets line up with the chain's
/// existing coverage grid rather than inventing a second, incompatible one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GeoBucket {
    pub lat_q: i64,
    pub lon_q: i64,
}

/// Default grid resolution, mirrors `btcpc_types::COVERAGE_GRID_RESOLUTION`.
/// Kept as a local constant (not re-exported from btcpc-types) because that
/// constant lives on the `CoverageReport`-specific path; Verasens uses the
/// same *value* by convention, not the same *item*.
pub const GRID_RESOLUTION: i64 = 1000;

impl GeoBucket {
    pub fn from_lat_lon(lat: f64, lon: f64) -> Self {
        Self::from_lat_lon_res(lat, lon, GRID_RESOLUTION)
    }

    pub fn from_lat_lon_res(lat: f64, lon: f64, resolution: i64) -> Self {
        GeoBucket {
            lat_q: (lat * resolution as f64).floor() as i64,
            lon_q: (lon * resolution as f64).floor() as i64,
        }
    }

    /// Stable string key, e.g. for use in HTTP query params / map keys.
    pub fn key(&self) -> String {
        format!("{}:{}", self.lat_q, self.lon_q)
    }

    pub fn parse_key(s: &str) -> Option<Self> {
        let (a, b) = s.split_once(':')?;
        Some(GeoBucket {
            lat_q: a.parse().ok()?,
            lon_q: b.parse().ok()?,
        })
    }
}

/// Epoch-based time window. `EPOCH_DURATION_S = 30.0` on chain; a window
/// groups `window_epochs` consecutive epochs into one bucket so queries can
/// ask for coarser granularity than a single 30s epoch (e.g. 1 hour = 120
/// epochs) without the aggregator re-bucketing on every query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TimeWindow {
    /// Index of the window, i.e. `epoch / window_epochs`.
    pub index: u64,
    /// Width of the window in epochs.
    pub window_epochs: u64,
}

/// Default window width: 120 epochs * 30s/epoch = 3600s = 1 hour.
pub const DEFAULT_WINDOW_EPOCHS: u64 = 120;

impl TimeWindow {
    pub fn from_epoch(epoch: u64, window_epochs: u64) -> Self {
        let window_epochs = window_epochs.max(1);
        TimeWindow {
            index: epoch / window_epochs,
            window_epochs,
        }
    }

    pub fn start_epoch(&self) -> u64 {
        self.index * self.window_epochs
    }

    pub fn end_epoch(&self) -> u64 {
        self.start_epoch() + self.window_epochs
    }

    pub fn key(&self) -> String {
        format!("{}:{}", self.window_epochs, self.index)
    }
}

/// The grouping key the aggregation index is keyed by: sensor class + geo
/// bucket + time window. `geo` is `None` when the reading carried no
/// lat/lon (e.g. an NFC/IR/RFID reading with no location attached) — those
/// readings are grouped under a class+time bucket with no geography.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GroupKey {
    pub class: String,
    pub geo: Option<GeoBucket>,
    pub window: TimeWindow,
}

/// A single contributing reading, fully parsed out of a chain `SensorReading`
/// entry's `metadata` per the ratified JSON convention.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedReading {
    pub sensor_id: String,
    pub owner: String,
    pub epoch: u64,
    pub value: f64,
    pub data_hash: String,
    pub class: SensorClass,
    pub schema: String,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub ts_ms: Option<i64>,
    pub payload: serde_json::Value,
    /// True only if this reading arrived through the signed path — see
    /// `ingest::IngestSource`. Unverified readings are still ingested and
    /// counted, but must be visibly downweighted / excluded from paid
    /// answers per the Phase 1.2 audit's trust rule.
    pub verified: bool,
}

impl ParsedReading {
    pub fn geo_bucket(&self) -> Option<GeoBucket> {
        match (self.lat, self.lon) {
            (Some(lat), Some(lon)) => Some(GeoBucket::from_lat_lon(lat, lon)),
            _ => None,
        }
    }

    pub fn group_key(&self, window_epochs: u64) -> GroupKey {
        GroupKey {
            class: self.class.as_str().to_string(),
            geo: self.geo_bucket(),
            window: TimeWindow::from_epoch(self.epoch, window_epochs),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geo_bucket_key_round_trips() {
        let b = GeoBucket::from_lat_lon(53.349, -6.260);
        let s = b.key();
        assert_eq!(GeoBucket::parse_key(&s), Some(b));
    }

    #[test]
    fn geo_bucket_parse_key_rejects_garbage() {
        assert_eq!(GeoBucket::parse_key("not-a-key"), None);
        assert_eq!(GeoBucket::parse_key("1:2:3"), None);
        assert_eq!(GeoBucket::parse_key("abc:def"), None);
    }

    #[test]
    fn time_window_key_encodes_width_and_index() {
        let w = TimeWindow::from_epoch(250, 120);
        assert_eq!(w.index, 2);
        assert_eq!(w.start_epoch(), 240);
        assert_eq!(w.end_epoch(), 360);
        assert_eq!(w.key(), "120:2");
    }

    #[test]
    fn sensor_class_display_matches_convention_strings() {
        for (c, s) in [
            (SensorClass::Gnss, "gnss"),
            (SensorClass::Subghz, "subghz"),
            (SensorClass::Ir, "ir"),
            (SensorClass::Nfc, "nfc"),
            (SensorClass::Rfid, "rfid"),
            (SensorClass::Ble, "ble"),
            (SensorClass::HeliumWitness, "helium_witness"),
        ] {
            assert_eq!(c.as_str(), s);
            assert_eq!(c.to_string(), s);
            assert_eq!(SensorClass::parse(s), Some(c));
        }
        assert_eq!(SensorClass::parse("bogus"), None);
    }
}

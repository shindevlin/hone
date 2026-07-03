//! Ingest: turns raw `SensorReading` chain entries into `ParsedReading`s the
//! aggregation index can consume.
//!
//! Deliberately split from the aggregation index (`agg.rs`) so ingest can be
//! unit-tested with hand-built entries/JSON, with no live chain required.
//! The live-chain adapter (polling a node's HTTP API or a gossip feed) is a
//! thin layer on top of `parse_reading` / `ingest_batch` — see `chain_feed.rs`.

use crate::model::{ParsedReading, SensorClass};
use btcpc_types::LedgerEntry;
use serde::{Deserialize, Serialize};

/// Where a `SensorReading` came from, and thus how much we trust it.
///
/// v1 trust model (per the Phase 1.2 audit's ratified rule): a reading
/// counts as VERIFIED only if it was signed by a registered device/posting
/// key. We do not re-verify ed25519 signatures here (that requires chain
/// internals — the account's registered posting key plus the canonical
/// signing message construction that only `tx.rs`/`check_signature` has).
/// Instead we trust the boundary: entries pulled from a node's `/api` after
/// the node itself has run them through `validate_and_apply` (which enforces
/// `check_signature` once an owner has a posting key registered) are
/// authoritative about whether the entry passed signature checks. This
/// adapter records that verdict rather than re-deriving it.
///
/// Concretely:
/// - `ChainConfirmed { signed }` — entry came from a sealed/applied chain
///   state (e.g. the node's block/entry API). `signed` reflects whether
///   `signed_by` was non-empty (bootstrap-skip unsigned readings are
///   ingested as unverified even though the chain accepted them).
/// - `Unconfirmed` — entry arrived out-of-band (e.g. raw gossip, a batch
///   file, a test fixture) with no chain confirmation at all. Always
///   unverified regardless of `signed_by`, since we have no proof the
///   node's signature check ever ran on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IngestSource {
    ChainConfirmed { signed: bool },
    Unconfirmed,
}

impl IngestSource {
    fn is_verified(&self) -> bool {
        matches!(self, IngestSource::ChainConfirmed { signed: true })
    }
}

/// Errors that can occur while parsing a `SensorReading` into the internal
/// model. Malformed metadata is common (readings that don't follow the
/// convention) so this is a recoverable, per-reading error, not a panic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("entry is not a SensorReading variant")]
    WrongVariant,
    #[error("metadata is missing (readings with no metadata cannot be classified/grouped)")]
    MissingMetadata,
    #[error("metadata.class is missing or not a recognized sensor class")]
    MissingOrUnknownClass,
    #[error("metadata.schema is missing")]
    MissingSchema,
    #[error("metadata.payload is missing or not a JSON object")]
    MissingPayload,
}

/// Parse one `LedgerEntry::SensorReading` into a `ParsedReading`, per the
/// ratified JSON convention (docs/PLATFORM_PRD.md Phase 1.2):
/// `metadata = { class, schema, lat?, lon?, ts_ms?, payload }`.
pub fn parse_reading(entry: &LedgerEntry, source: IngestSource) -> Result<ParsedReading, ParseError> {
    let LedgerEntry::SensorReading {
        sensor_id,
        owner,
        epoch,
        value,
        data_hash,
        metadata,
        signed_by: _,
    } = entry
    else {
        return Err(ParseError::WrongVariant);
    };

    let meta = metadata.as_ref().ok_or(ParseError::MissingMetadata)?;
    let obj = meta.as_object().ok_or(ParseError::MissingMetadata)?;

    let class = obj
        .get("class")
        .and_then(|v| v.as_str())
        .and_then(SensorClass::parse)
        .ok_or(ParseError::MissingOrUnknownClass)?;

    let schema = obj
        .get("schema")
        .and_then(|v| v.as_str())
        .ok_or(ParseError::MissingSchema)?
        .to_string();

    let payload = obj
        .get("payload")
        .filter(|v| v.is_object())
        .cloned()
        .ok_or(ParseError::MissingPayload)?;

    let lat = obj.get("lat").and_then(|v| v.as_f64());
    let lon = obj.get("lon").and_then(|v| v.as_f64());
    let ts_ms = obj.get("ts_ms").and_then(|v| v.as_i64());

    Ok(ParsedReading {
        sensor_id: sensor_id.clone(),
        owner: owner.clone(),
        epoch: *epoch,
        value: *value,
        data_hash: data_hash.clone(),
        class,
        schema,
        lat,
        lon,
        ts_ms,
        payload,
        verified: source.is_verified(),
    })
}

/// Derive an `IngestSource` from a `LedgerEntry::SensorReading`'s own
/// `signed_by` field, for callers that already trust the entry came from
/// chain-confirmed state (i.e. they got it from a node that ran
/// `validate_and_apply`). This does NOT re-verify any signature bytes —
/// see the `IngestSource` doc comment for why that's out of scope for v1.
pub fn source_from_entry(entry: &LedgerEntry) -> IngestSource {
    match entry {
        LedgerEntry::SensorReading { signed_by, .. } => IngestSource::ChainConfirmed {
            signed: !signed_by.is_empty(),
        },
        _ => IngestSource::Unconfirmed,
    }
}

/// Wire-format-agnostic batch item: pairs a raw entry with an explicit trust
/// source, for callers (tests, adapters) that want to control provenance
/// independent of the entry's own `signed_by` field.
#[derive(Debug, Clone)]
pub struct IngestItem {
    pub entry: LedgerEntry,
    pub source: IngestSource,
}

impl IngestItem {
    /// Build an item that trusts the entry's own `signed_by` field, i.e.
    /// "this came from confirmed chain state."
    pub fn chain_confirmed(entry: LedgerEntry) -> Self {
        let source = source_from_entry(&entry);
        IngestItem { entry, source }
    }

    /// Build an item explicitly marked unconfirmed (e.g. raw gossip/batch
    /// input with no chain confirmation yet).
    pub fn unconfirmed(entry: LedgerEntry) -> Self {
        IngestItem {
            entry,
            source: IngestSource::Unconfirmed,
        }
    }
}

/// Parse a batch, collecting successes and (sensor_id-tagged) failures
/// separately so one malformed reading in a batch doesn't drop the rest.
#[derive(Debug, Default)]
pub struct BatchParseResult {
    pub parsed: Vec<ParsedReading>,
    pub errors: Vec<(String, ParseError)>,
}

pub fn parse_batch(items: &[IngestItem]) -> BatchParseResult {
    let mut result = BatchParseResult::default();
    for item in items {
        match parse_reading(&item.entry, item.source) {
            Ok(pr) => result.parsed.push(pr),
            Err(e) => {
                let sid = match &item.entry {
                    LedgerEntry::SensorReading { sensor_id, .. } => sensor_id.clone(),
                    _ => "<non-sensor-reading>".to_string(),
                };
                result.errors.push((sid, e));
            }
        }
    }
    result
}

/// Minimal JSON shape used by the live-chain HTTP adapter (`chain_feed.rs`)
/// when a node exposes sensor readings as a plain JSON array rather than
/// borsh/serde `LedgerEntry` directly. Kept separate from `LedgerEntry` so
/// this crate never has to assume a specific node API response envelope —
/// callers can `serde_json::from_value` into this and hand it to
/// `raw_reading_to_entry`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawSensorReading {
    pub sensor_id: String,
    pub owner: String,
    pub epoch: u64,
    pub value: f64,
    pub data_hash: String,
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub signed_by: String,
}

pub fn raw_reading_to_entry(raw: RawSensorReading) -> LedgerEntry {
    LedgerEntry::SensorReading {
        sensor_id: raw.sensor_id,
        owner: raw.owner,
        epoch: raw.epoch,
        value: raw.value,
        data_hash: raw.data_hash,
        metadata: raw.metadata,
        signed_by: raw.signed_by,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gnss_entry(sensor_id: &str, owner: &str, epoch: u64, signed_by: &str) -> LedgerEntry {
        LedgerEntry::SensorReading {
            sensor_id: sensor_id.to_string(),
            owner: owner.to_string(),
            epoch,
            value: 3.5,
            data_hash: "deadbeef".to_string(),
            metadata: Some(json!({
                "class": "gnss",
                "schema": "1",
                "lat": 53.349,
                "lon": -6.260,
                "ts_ms": 1_777_633_200_000i64,
                "payload": {
                    "alt_m": 12.0,
                    "h_accuracy_m": 3.5,
                    "sat_count": 9
                }
            })),
            signed_by: signed_by.to_string(),
        }
    }

    #[test]
    fn parses_valid_gnss_reading_as_verified_when_signed() {
        let entry = gnss_entry("sensor-1", "alice", 100, "alice");
        let source = source_from_entry(&entry);
        let parsed = parse_reading(&entry, source).expect("should parse");
        assert_eq!(parsed.class, SensorClass::Gnss);
        assert_eq!(parsed.lat, Some(53.349));
        assert_eq!(parsed.lon, Some(-6.260));
        assert!(parsed.verified);
    }

    #[test]
    fn unsigned_reading_is_unverified_even_if_chain_confirmed() {
        let entry = gnss_entry("sensor-2", "bob", 100, "");
        let source = source_from_entry(&entry);
        let parsed = parse_reading(&entry, source).expect("should parse");
        assert!(!parsed.verified);
    }

    #[test]
    fn unconfirmed_source_is_always_unverified_even_if_signed_by_present() {
        let entry = gnss_entry("sensor-3", "carol", 100, "carol");
        let parsed = parse_reading(&entry, IngestSource::Unconfirmed).expect("should parse");
        assert!(!parsed.verified, "unconfirmed source must never be trusted as verified");
    }

    #[test]
    fn missing_metadata_is_rejected() {
        let entry = LedgerEntry::SensorReading {
            sensor_id: "sensor-4".into(),
            owner: "dave".into(),
            epoch: 1,
            value: 1.0,
            data_hash: "x".into(),
            metadata: None,
            signed_by: "dave".into(),
        };
        let err = parse_reading(&entry, IngestSource::Unconfirmed).unwrap_err();
        assert_eq!(err, ParseError::MissingMetadata);
    }

    #[test]
    fn missing_class_is_rejected() {
        let entry = LedgerEntry::SensorReading {
            sensor_id: "sensor-5".into(),
            owner: "erin".into(),
            epoch: 1,
            value: 1.0,
            data_hash: "x".into(),
            metadata: Some(json!({"schema": "1", "payload": {}})),
            signed_by: "erin".into(),
        };
        let err = parse_reading(&entry, IngestSource::Unconfirmed).unwrap_err();
        assert_eq!(err, ParseError::MissingOrUnknownClass);
    }

    #[test]
    fn missing_payload_is_rejected() {
        let entry = LedgerEntry::SensorReading {
            sensor_id: "sensor-6".into(),
            owner: "frank".into(),
            epoch: 1,
            value: 1.0,
            data_hash: "x".into(),
            metadata: Some(json!({"class": "ble", "schema": "1"})),
            signed_by: "frank".into(),
        };
        let err = parse_reading(&entry, IngestSource::Unconfirmed).unwrap_err();
        assert_eq!(err, ParseError::MissingPayload);
    }

    #[test]
    fn batch_parse_keeps_good_readings_and_reports_bad_ones() {
        let good = gnss_entry("sensor-7", "gina", 5, "gina");
        let bad = LedgerEntry::SensorReading {
            sensor_id: "sensor-8".into(),
            owner: "hank".into(),
            epoch: 5,
            value: 1.0,
            data_hash: "x".into(),
            metadata: None,
            signed_by: "".into(),
        };
        let items = vec![
            IngestItem::chain_confirmed(good),
            IngestItem::chain_confirmed(bad),
        ];
        let result = parse_batch(&items);
        assert_eq!(result.parsed.len(), 1);
        assert_eq!(result.errors.len(), 1);
        assert_eq!(result.errors[0].0, "sensor-8");
    }

    #[test]
    fn raw_reading_round_trips_through_json() {
        let raw = RawSensorReading {
            sensor_id: "sensor-9".into(),
            owner: "ivan".into(),
            epoch: 42,
            value: 7.0,
            data_hash: "abc".into(),
            metadata: Some(json!({
                "class": "subghz",
                "schema": "1",
                "payload": {"freq_hz": 433_920_000, "rssi_dbm": -70}
            })),
            signed_by: "ivan".into(),
        };
        let s = serde_json::to_string(&raw).unwrap();
        let back: RawSensorReading = serde_json::from_str(&s).unwrap();
        let entry = raw_reading_to_entry(back);
        let parsed = parse_reading(&entry, IngestSource::ChainConfirmed { signed: true }).unwrap();
        assert_eq!(parsed.class, SensorClass::Subghz);
        assert!(parsed.verified);
    }
}

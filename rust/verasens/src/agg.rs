//! The aggregation index: pure, in-memory, unit-testable core.
//!
//! Keyed by `GroupKey` (sensor class + geo bucket + time window). Each group
//! tracks running value statistics (count/mean/min/max/percentiles),
//! verified-vs-unverified counts, and per-reading provenance (sensor_id,
//! owner, data_hash) so query responses can answer "what fed this number"
//! for the Phase 1.4 settlement bridge / dashboard.
//!
//! Deliberately has zero knowledge of Axum, HTTP, or the chain wire format —
//! only `model::ParsedReading` in, `GroupSnapshot` out. This is what makes it
//! testable without spinning up a server or a chain.

use crate::model::{GroupKey, ParsedReading};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One contributing reading's provenance, as exposed in query results.
/// Kept intentionally small (no raw payload) — this is provenance, not a
/// raw-reading dump, per the "not a passthrough" requirement.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Provenance {
    pub sensor_id: String,
    pub owner: String,
    pub data_hash: String,
    pub epoch: u64,
    pub verified: bool,
}

/// Mutable accumulator for one group. Not exposed directly — snapshotted
/// into `GroupSnapshot` for queries so percentile computation only happens
/// on read, not on every ingest.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct GroupAccumulator {
    /// All values seen so far, kept to compute percentiles on demand.
    /// For v1 this is fine (in-memory, no persistence yet); if group sizes
    /// grow large enough to matter, this is the spot to swap in a
    /// streaming quantile sketch (e.g. t-digest) without touching callers.
    values: Vec<f64>,
    verified_count: u64,
    unverified_count: u64,
    provenance: Vec<Provenance>,
}

impl GroupAccumulator {
    fn add(&mut self, reading: &ParsedReading) {
        self.values.push(reading.value);
        if reading.verified {
            self.verified_count += 1;
        } else {
            self.unverified_count += 1;
        }
        self.provenance.push(Provenance {
            sensor_id: reading.sensor_id.clone(),
            owner: reading.owner.clone(),
            data_hash: reading.data_hash.clone(),
            epoch: reading.epoch,
            verified: reading.verified,
        });
    }

    fn snapshot(&self, key: &GroupKey) -> GroupSnapshot {
        GroupSnapshot {
            key: key.clone(),
            count: self.values.len() as u64,
            verified_count: self.verified_count,
            unverified_count: self.unverified_count,
            stats: ValueStats::from_values(&self.values),
            provenance: self.provenance.clone(),
        }
    }
}

/// Value statistics for a group's readings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ValueStats {
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    pub p50: f64,
    pub p95: f64,
}

impl ValueStats {
    fn from_values(values: &[f64]) -> Option<Self> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let sum: f64 = sorted.iter().sum();
        let mean = sum / sorted.len() as f64;
        let min = sorted[0];
        let max = sorted[sorted.len() - 1];

        Some(ValueStats {
            mean,
            min,
            max,
            p50: percentile(&sorted, 0.50),
            p95: percentile(&sorted, 0.95),
        })
    }
}

/// Nearest-rank percentile over an already-sorted slice.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = (p * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

/// A read-only snapshot of one group, as returned by queries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroupSnapshot {
    pub key: GroupKey,
    pub count: u64,
    pub verified_count: u64,
    pub unverified_count: u64,
    /// `None` only if `count == 0`, which shouldn't happen for a stored
    /// group in practice (groups are created on first insert) but is kept
    /// `Option` to avoid a bogus all-zero stats block.
    pub stats: Option<ValueStats>,
    pub provenance: Vec<Provenance>,
}

/// The aggregation index. Idempotent by design: re-ingesting the exact same
/// `ParsedReading` (same sensor_id + epoch + data_hash) is detected and
/// skipped via `seen` dedup, so a crash-and-replay ingest source (the
/// self-heal / restart case owned by the sibling Tests PRD item) doesn't
/// double-count — this dedup is the mechanism that item's tests are
/// expected to exercise end-to-end.
#[derive(Debug, Default)]
pub struct AggregationIndex {
    groups: BTreeMap<GroupKey, GroupAccumulator>,
    /// Dedup set: (sensor_id, epoch, data_hash) already ingested.
    seen: std::collections::HashSet<(String, u64, String)>,
    /// Width, in epochs, used to bucket readings into time windows. See
    /// `model::TimeWindow`.
    window_epochs: u64,
    /// Total readings ingested (including duplicates that were skipped),
    /// for observability.
    total_ingested: u64,
    total_duplicates_skipped: u64,
    total_rejected: u64,
}

impl AggregationIndex {
    pub fn new(window_epochs: u64) -> Self {
        AggregationIndex {
            groups: BTreeMap::new(),
            seen: std::collections::HashSet::new(),
            window_epochs: window_epochs.max(1),
            total_ingested: 0,
            total_duplicates_skipped: 0,
            total_rejected: 0,
        }
    }

    /// Ingest one already-parsed reading. Returns `true` if it was newly
    /// counted, `false` if it was a duplicate (already-seen) and skipped.
    pub fn ingest(&mut self, reading: &ParsedReading) -> bool {
        let dedup_key = (
            reading.sensor_id.clone(),
            reading.epoch,
            reading.data_hash.clone(),
        );
        if self.seen.contains(&dedup_key) {
            self.total_duplicates_skipped += 1;
            return false;
        }
        self.seen.insert(dedup_key);

        let key = reading.group_key(self.window_epochs);
        self.groups.entry(key).or_default().add(reading);
        self.total_ingested += 1;
        true
    }

    /// Ingest a batch, returning (newly_counted, duplicates_skipped).
    pub fn ingest_batch(&mut self, readings: &[ParsedReading]) -> (u64, u64) {
        let mut newly = 0;
        let mut dupes = 0;
        for r in readings {
            if self.ingest(r) {
                newly += 1;
            } else {
                dupes += 1;
            }
        }
        (newly, dupes)
    }

    /// Record a rejected (unparseable) reading for observability. Does not
    /// affect any group.
    pub fn record_rejected(&mut self, count: u64) {
        self.total_rejected += count;
    }

    /// Exact lookup for one group.
    pub fn get(&self, key: &GroupKey) -> Option<GroupSnapshot> {
        self.groups.get(key).map(|acc| acc.snapshot(key))
    }

    /// Query groups matching an optional class / geo bucket / epoch range
    /// filter. This is the backbone of the HTTP query API.
    pub fn query(&self, filter: &QueryFilter) -> Vec<GroupSnapshot> {
        self.groups
            .iter()
            .filter(|(key, _)| filter.matches(key))
            .map(|(key, acc)| acc.snapshot(key))
            .collect()
    }

    pub fn stats(&self) -> IndexStats {
        IndexStats {
            group_count: self.groups.len() as u64,
            total_ingested: self.total_ingested,
            total_duplicates_skipped: self.total_duplicates_skipped,
            total_rejected: self.total_rejected,
        }
    }

    /// Build the on-disk snapshot representation. Kept as a separate type
    /// (`PersistedIndex`) rather than deriving Serialize directly on
    /// `AggregationIndex` so the disk format is an explicit, versioned
    /// contract independent of any future in-memory field changes — the
    /// HTTP query API (`api.rs`/`GroupSnapshot`) is unaffected either way.
    fn to_persisted(&self) -> PersistedIndex {
        PersistedIndex {
            format_version: PERSISTED_FORMAT_VERSION,
            window_epochs: self.window_epochs,
            groups: self.groups.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            seen: self.seen.iter().cloned().collect(),
            total_ingested: self.total_ingested,
            total_duplicates_skipped: self.total_duplicates_skipped,
            total_rejected: self.total_rejected,
        }
    }

    /// Reconstruct an index from a previously-persisted snapshot. Restores
    /// both the aggregation groups AND the dedup `seen` set, so readings
    /// already folded in before a restart are recognized as duplicates
    /// (not double-counted) if the ingest source replays them.
    fn from_persisted(p: PersistedIndex) -> Self {
        AggregationIndex {
            groups: p.groups.into_iter().collect(),
            seen: p.seen.into_iter().collect(),
            window_epochs: p.window_epochs.max(1),
            total_ingested: p.total_ingested,
            total_duplicates_skipped: p.total_duplicates_skipped,
            total_rejected: p.total_rejected,
        }
    }

    /// Persist the current index state to `path`, atomically.
    ///
    /// Writes to a sibling temp file, fsyncs its contents, then renames it
    /// over `path` (rename is atomic on the same filesystem on both Unix and
    /// Windows), so a crash mid-write never leaves `path` holding a
    /// truncated/corrupt snapshot — readers either see the old complete file
    /// or the new complete file, never a partial one.
    pub fn save_to_file(&self, path: &std::path::Path) -> std::io::Result<()> {
        let persisted = self.to_persisted();
        let bytes = serde_json::to_vec(&persisted)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
        if let Some(dir) = dir {
            std::fs::create_dir_all(dir)?;
        }

        // Unique temp filename so concurrent saves (shouldn't normally
        // happen, but tests may run in parallel against distinct
        // AggregationIndex instances sharing a tmp dir) don't clobber each
        // other before the rename.
        let tmp_path = {
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "index".to_string());
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let tmp_name = format!(".{file_name}.tmp.{}.{unique}", std::process::id());
            match path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => parent.join(tmp_name),
                _ => std::path::PathBuf::from(tmp_name),
            }
        };

        {
            let mut f = std::fs::File::create(&tmp_path)?;
            use std::io::Write;
            f.write_all(&bytes)?;
            f.sync_all()?;
        }

        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }

    /// Load a previously-persisted index from `path`. Returns `Ok(None)` if
    /// the file does not exist yet (fresh start, e.g. first boot — this is
    /// not an error). Returns `Err` if the file exists but cannot be read or
    /// parsed, since a corrupt persisted file silently treated as "empty"
    /// would itself be a silent data-loss bug.
    pub fn load_from_file(path: &std::path::Path) -> std::io::Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(path)?;
        let persisted: PersistedIndex = serde_json::from_slice(&bytes)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Some(Self::from_persisted(persisted)))
    }
}

/// On-disk snapshot format for `AggregationIndex`. `format_version` lets a
/// future v3 detect and migrate an older on-disk file instead of silently
/// misreading it; loading is expected to stay API-compatible (same HTTP
/// query contract) even as this internal format evolves.
const PERSISTED_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedIndex {
    format_version: u32,
    window_epochs: u64,
    groups: Vec<(GroupKey, GroupAccumulator)>,
    /// Dedup set entries, flattened to a Vec for JSON (JSON object/map keys
    /// must be strings; a tuple key doesn't serialize as a map key without
    /// extra ceremony, so a plain list round-trips more simply).
    seen: Vec<(String, u64, String)>,
    total_ingested: u64,
    total_duplicates_skipped: u64,
    total_rejected: u64,
}

/// Observability counters for the whole index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub group_count: u64,
    pub total_ingested: u64,
    pub total_duplicates_skipped: u64,
    pub total_rejected: u64,
}

/// Filter for `AggregationIndex::query`. All fields are optional (`None` =
/// match anything on that dimension).
#[derive(Debug, Clone, Default)]
pub struct QueryFilter {
    pub class: Option<String>,
    pub geo: Option<crate::model::GeoBucket>,
    pub window_index: Option<u64>,
    /// Epoch range filter, applied against the window's start epoch — lets
    /// callers ask for "windows overlapping [from, to)" without knowing
    /// window indices.
    pub epoch_from: Option<u64>,
    pub epoch_to: Option<u64>,
}

impl QueryFilter {
    fn matches(&self, key: &GroupKey) -> bool {
        if let Some(class) = &self.class {
            if &key.class != class {
                return false;
            }
        }
        if let Some(geo) = &self.geo {
            if key.geo.as_ref() != Some(geo) {
                return false;
            }
        }
        if let Some(idx) = self.window_index {
            if key.window.index != idx {
                return false;
            }
        }
        if let Some(from) = self.epoch_from {
            if key.window.end_epoch() <= from {
                return false;
            }
        }
        if let Some(to) = self.epoch_to {
            if key.window.start_epoch() >= to {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GeoBucket, SensorClass};

    #[allow(clippy::too_many_arguments)] // test-only fixture builder; positional args read fine at call sites below
    fn reading(
        sensor_id: &str,
        owner: &str,
        epoch: u64,
        value: f64,
        class: SensorClass,
        lat: Option<f64>,
        lon: Option<f64>,
        verified: bool,
        data_hash: &str,
    ) -> ParsedReading {
        ParsedReading {
            sensor_id: sensor_id.to_string(),
            owner: owner.to_string(),
            epoch,
            value,
            data_hash: data_hash.to_string(),
            class,
            schema: "1".to_string(),
            lat,
            lon,
            ts_ms: None,
            payload: serde_json::json!({}),
            verified,
        }
    }

    #[test]
    fn ingest_groups_by_class_geo_and_window() {
        let mut idx = AggregationIndex::new(120);
        idx.ingest(&reading(
            "s1", "alice", 10, 5.0, SensorClass::Gnss, Some(53.349), Some(-6.260), true, "h1",
        ));
        idx.ingest(&reading(
            "s2", "bob", 20, 7.0, SensorClass::Gnss, Some(53.349), Some(-6.260), true, "h2",
        ));
        // Different class -> different group even with same geo/time.
        idx.ingest(&reading(
            "s3", "carol", 15, 100.0, SensorClass::Subghz, Some(53.349), Some(-6.260), true, "h3",
        ));

        let groups = idx.query(&QueryFilter::default());
        assert_eq!(groups.len(), 2, "gnss and subghz should be separate groups");

        let gnss_group = idx
            .query(&QueryFilter {
                class: Some("gnss".to_string()),
                ..Default::default()
            });
        assert_eq!(gnss_group.len(), 1);
        assert_eq!(gnss_group[0].count, 2);
        let stats = gnss_group[0].stats.as_ref().unwrap();
        assert_eq!(stats.mean, 6.0);
        assert_eq!(stats.min, 5.0);
        assert_eq!(stats.max, 7.0);
    }

    #[test]
    fn different_geo_buckets_stay_separate() {
        let mut idx = AggregationIndex::new(120);
        idx.ingest(&reading(
            "s1", "alice", 10, 1.0, SensorClass::Gnss, Some(53.349), Some(-6.260), true, "h1",
        ));
        // Far away lat/lon -> different grid cell.
        idx.ingest(&reading(
            "s2", "bob", 10, 2.0, SensorClass::Gnss, Some(40.7128), Some(-74.0060), true, "h2",
        ));
        let groups = idx.query(&QueryFilter::default());
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn readings_without_lat_lon_group_under_no_geo() {
        let mut idx = AggregationIndex::new(120);
        idx.ingest(&reading("s1", "alice", 10, 1.0, SensorClass::Nfc, None, None, true, "h1"));
        idx.ingest(&reading("s2", "bob", 10, 2.0, SensorClass::Nfc, None, None, true, "h2"));
        let groups = idx.query(&QueryFilter {
            class: Some("nfc".to_string()),
            ..Default::default()
        });
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].key.geo, None);
        assert_eq!(groups[0].count, 2);
    }

    #[test]
    fn different_time_windows_stay_separate() {
        let mut idx = AggregationIndex::new(120); // 120-epoch windows
        idx.ingest(&reading(
            "s1", "alice", 10, 1.0, SensorClass::Ble, None, None, true, "h1",
        ));
        idx.ingest(&reading(
            "s2", "bob", 200, 2.0, SensorClass::Ble, None, None, true, "h2",
        )); // window index 1
        let groups = idx.query(&QueryFilter {
            class: Some("ble".to_string()),
            ..Default::default()
        });
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn verified_and_unverified_are_tracked_separately() {
        let mut idx = AggregationIndex::new(120);
        idx.ingest(&reading("s1", "alice", 10, 1.0, SensorClass::Ir, None, None, true, "h1"));
        idx.ingest(&reading("s2", "bob", 10, 2.0, SensorClass::Ir, None, None, false, "h2"));
        idx.ingest(&reading("s3", "carol", 10, 3.0, SensorClass::Ir, None, None, false, "h3"));

        let groups = idx.query(&QueryFilter {
            class: Some("ir".to_string()),
            ..Default::default()
        });
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].count, 3);
        assert_eq!(groups[0].verified_count, 1);
        assert_eq!(groups[0].unverified_count, 2);
    }

    #[test]
    fn duplicate_reading_same_sensor_epoch_hash_is_not_double_counted() {
        let mut idx = AggregationIndex::new(120);
        let r = reading("s1", "alice", 10, 1.0, SensorClass::Gnss, Some(1.0), Some(1.0), true, "h1");
        assert!(idx.ingest(&r));
        assert!(!idx.ingest(&r), "exact duplicate must be rejected");

        let groups = idx.query(&QueryFilter::default());
        assert_eq!(groups[0].count, 1);
        let stats = idx.stats();
        assert_eq!(stats.total_ingested, 1);
        assert_eq!(stats.total_duplicates_skipped, 1);
    }

    #[test]
    fn provenance_lists_every_contributing_reading() {
        let mut idx = AggregationIndex::new(120);
        idx.ingest(&reading("s1", "alice", 10, 1.0, SensorClass::Gnss, Some(1.0), Some(1.0), true, "h1"));
        idx.ingest(&reading("s2", "bob", 10, 2.0, SensorClass::Gnss, Some(1.0), Some(1.0), false, "h2"));

        let groups = idx.query(&QueryFilter::default());
        let prov = &groups[0].provenance;
        assert_eq!(prov.len(), 2);
        assert!(prov.iter().any(|p| p.sensor_id == "s1" && p.owner == "alice" && p.verified));
        assert!(prov.iter().any(|p| p.sensor_id == "s2" && p.owner == "bob" && !p.verified));
    }

    #[test]
    fn geo_bucket_query_filter_matches_exact_cell_only() {
        let mut idx = AggregationIndex::new(120);
        idx.ingest(&reading(
            "s1", "alice", 10, 1.0, SensorClass::Gnss, Some(53.349), Some(-6.260), true, "h1",
        ));
        let target = GeoBucket::from_lat_lon(53.349, -6.260);
        let groups = idx.query(&QueryFilter {
            geo: Some(target),
            ..Default::default()
        });
        assert_eq!(groups.len(), 1);

        let other = GeoBucket::from_lat_lon(0.0, 0.0);
        let empty = idx.query(&QueryFilter {
            geo: Some(other),
            ..Default::default()
        });
        assert!(empty.is_empty());
    }

    #[test]
    fn epoch_range_filter_selects_overlapping_windows() {
        let mut idx = AggregationIndex::new(10); // 10-epoch windows
        idx.ingest(&reading("s1", "a", 5, 1.0, SensorClass::Ble, None, None, true, "h1")); // window 0: [0,10)
        idx.ingest(&reading("s2", "b", 15, 2.0, SensorClass::Ble, None, None, true, "h2")); // window 1: [10,20)
        idx.ingest(&reading("s3", "c", 25, 3.0, SensorClass::Ble, None, None, true, "h3")); // window 2: [20,30)

        let groups = idx.query(&QueryFilter {
            class: Some("ble".to_string()),
            epoch_from: Some(10),
            epoch_to: Some(20),
            ..Default::default()
        });
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].key.window.index, 1);
    }

    #[test]
    fn percentiles_are_reasonable_over_ordered_values() {
        let mut idx = AggregationIndex::new(120);
        for i in 1..=10 {
            idx.ingest(&reading(
                &format!("s{i}"),
                "alice",
                10,
                i as f64,
                SensorClass::Gnss,
                Some(1.0),
                Some(1.0),
                true,
                &format!("h{i}"),
            ));
        }
        let groups = idx.query(&QueryFilter::default());
        let stats = groups[0].stats.as_ref().unwrap();
        assert_eq!(stats.min, 1.0);
        assert_eq!(stats.max, 10.0);
        assert!((stats.mean - 5.5).abs() < 1e-9);
        // p50 of [1..10] nearest-rank should land in the middle.
        assert!(stats.p50 >= 5.0 && stats.p50 <= 6.0);
        assert!(stats.p95 >= 9.0);
    }

    /// 3+-sensor-class end-to-end ingest scenario (PRD Phase 1.2 "Tests"
    /// item, gap 2): a single scenario ingests readings from three distinct
    /// sensor classes and asserts each aggregates into its own independent
    /// group with correct per-group stats, with zero cross-contamination
    /// between classes.
    #[test]
    fn three_plus_sensor_classes_aggregate_independently_in_one_scenario() {
        let mut idx = AggregationIndex::new(120);

        // Same geography + same time window for all three classes, on
        // purpose: geo/window alone must NOT merge readings across classes
        // — `class` is part of `GroupKey` and must be the deciding factor.
        let lat = Some(53.349);
        let lon = Some(-6.260);
        let epoch = 42;

        // gnss: 3 readings.
        idx.ingest(&reading("gnss-1", "alice", epoch, 10.0, SensorClass::Gnss, lat, lon, true, "g1"));
        idx.ingest(&reading("gnss-2", "alice", epoch, 20.0, SensorClass::Gnss, lat, lon, true, "g2"));
        idx.ingest(&reading("gnss-3", "bob", epoch, 30.0, SensorClass::Gnss, lat, lon, false, "g3"));

        // subghz: 2 readings.
        idx.ingest(&reading("sg-1", "carol", epoch, 100.0, SensorClass::Subghz, lat, lon, true, "s1"));
        idx.ingest(&reading("sg-2", "carol", epoch, 200.0, SensorClass::Subghz, lat, lon, true, "s2"));

        // nfc: 4 readings.
        idx.ingest(&reading("nfc-1", "dave", epoch, 1.0, SensorClass::Nfc, lat, lon, true, "n1"));
        idx.ingest(&reading("nfc-2", "dave", epoch, 2.0, SensorClass::Nfc, lat, lon, true, "n2"));
        idx.ingest(&reading("nfc-3", "erin", epoch, 3.0, SensorClass::Nfc, lat, lon, false, "n3"));
        idx.ingest(&reading("nfc-4", "erin", epoch, 4.0, SensorClass::Nfc, lat, lon, false, "n4"));

        // A fourth class thrown in too, to go beyond the minimum 3.
        idx.ingest(&reading("ble-1", "frank", epoch, 55.0, SensorClass::Ble, lat, lon, true, "b1"));

        let all_groups = idx.query(&QueryFilter::default());
        assert_eq!(all_groups.len(), 4, "each sensor class must form its own independent group");

        let idx_stats = idx.stats();
        assert_eq!(idx_stats.group_count, 4);
        assert_eq!(idx_stats.total_ingested, 10);
        assert_eq!(idx_stats.total_duplicates_skipped, 0);

        // gnss group: count/verified/unverified/stats correct, and no
        // contamination from other classes' values.
        let gnss = idx.query(&QueryFilter { class: Some("gnss".to_string()), ..Default::default() });
        assert_eq!(gnss.len(), 1);
        assert_eq!(gnss[0].count, 3);
        assert_eq!(gnss[0].verified_count, 2);
        assert_eq!(gnss[0].unverified_count, 1);
        let gnss_stats = gnss[0].stats.as_ref().unwrap();
        assert_eq!(gnss_stats.min, 10.0);
        assert_eq!(gnss_stats.max, 30.0);
        assert!((gnss_stats.mean - 20.0).abs() < 1e-9);

        // subghz group: independent count/stats, untouched by gnss/nfc/ble.
        let subghz = idx.query(&QueryFilter { class: Some("subghz".to_string()), ..Default::default() });
        assert_eq!(subghz.len(), 1);
        assert_eq!(subghz[0].count, 2);
        assert_eq!(subghz[0].verified_count, 2);
        assert_eq!(subghz[0].unverified_count, 0);
        let subghz_stats = subghz[0].stats.as_ref().unwrap();
        assert_eq!(subghz_stats.min, 100.0);
        assert_eq!(subghz_stats.max, 200.0);
        assert!((subghz_stats.mean - 150.0).abs() < 1e-9);

        // nfc group: independent count/stats.
        let nfc = idx.query(&QueryFilter { class: Some("nfc".to_string()), ..Default::default() });
        assert_eq!(nfc.len(), 1);
        assert_eq!(nfc[0].count, 4);
        assert_eq!(nfc[0].verified_count, 2);
        assert_eq!(nfc[0].unverified_count, 2);
        let nfc_stats = nfc[0].stats.as_ref().unwrap();
        assert_eq!(nfc_stats.min, 1.0);
        assert_eq!(nfc_stats.max, 4.0);
        assert!((nfc_stats.mean - 2.5).abs() < 1e-9);

        // ble group: the fourth class, also independent.
        let ble = idx.query(&QueryFilter { class: Some("ble".to_string()), ..Default::default() });
        assert_eq!(ble.len(), 1);
        assert_eq!(ble[0].count, 1);
        assert_eq!(ble[0].stats.as_ref().unwrap().mean, 55.0);

        // Cross-contamination check: no group's provenance lists a
        // sensor_id that belongs to a different class's readings.
        let gnss_ids: std::collections::HashSet<_> = ["gnss-1", "gnss-2", "gnss-3"].into_iter().collect();
        for p in &gnss[0].provenance {
            assert!(gnss_ids.contains(p.sensor_id.as_str()), "gnss group contaminated by {}", p.sensor_id);
        }
        let nfc_ids: std::collections::HashSet<_> = ["nfc-1", "nfc-2", "nfc-3", "nfc-4"].into_iter().collect();
        for p in &nfc[0].provenance {
            assert!(nfc_ids.contains(p.sensor_id.as_str()), "nfc group contaminated by {}", p.sensor_id);
        }
    }
}

/// Self-heal / restart persistence tests (PRD Phase 1.2 "Tests" item, gap
/// 1): a process restart (drop the in-memory `AggregationIndex`, reload it
/// from the persisted file) must not lose or double-count in-flight
/// readings, including when the post-restart ingest source replays some
/// readings the pre-restart index had already seen.
#[cfg(test)]
mod persistence_tests {
    use super::*;
    use crate::model::SensorClass;

    #[allow(clippy::too_many_arguments)]
    fn reading(
        sensor_id: &str,
        owner: &str,
        epoch: u64,
        value: f64,
        class: SensorClass,
        lat: Option<f64>,
        lon: Option<f64>,
        verified: bool,
        data_hash: &str,
    ) -> ParsedReading {
        ParsedReading {
            sensor_id: sensor_id.to_string(),
            owner: owner.to_string(),
            epoch,
            value,
            data_hash: data_hash.to_string(),
            class,
            schema: "1".to_string(),
            lat,
            lon,
            ts_ms: None,
            payload: serde_json::json!({}),
            verified,
        }
    }

    /// Unique temp file path per test run/thread, so parallel `cargo test`
    /// runs never collide on the same on-disk snapshot file.
    fn temp_state_path(tag: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "verasens-selfheal-test-{tag}-{}-{unique}.json",
            std::process::id()
        ))
    }

    /// RAII guard so the temp snapshot file is cleaned up even if an
    /// assertion panics mid-test.
    struct TempFileGuard(std::path::PathBuf);
    impl Drop for TempFileGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn batch_a() -> Vec<ParsedReading> {
        vec![
            reading("s1", "alice", 10, 1.0, SensorClass::Gnss, Some(1.0), Some(1.0), true, "h1"),
            reading("s2", "bob", 10, 2.0, SensorClass::Gnss, Some(1.0), Some(1.0), true, "h2"),
            reading("s3", "carol", 15, 100.0, SensorClass::Subghz, None, None, false, "h3"),
        ]
    }

    /// Readings ingested *after* the simulated restart: two overlap exactly
    /// with `batch_a` (same sensor_id+epoch+data_hash — as a crash-and-
    /// replay ingest source would resend unacknowledged items), and two are
    /// genuinely new.
    fn batch_b_overlapping_and_new() -> Vec<ParsedReading> {
        vec![
            // Exact duplicates of batch_a entries (replayed after "crash").
            reading("s1", "alice", 10, 1.0, SensorClass::Gnss, Some(1.0), Some(1.0), true, "h1"),
            reading("s3", "carol", 15, 100.0, SensorClass::Subghz, None, None, false, "h3"),
            // Genuinely new readings.
            reading("s4", "dave", 20, 3.0, SensorClass::Gnss, Some(1.0), Some(1.0), true, "h4"),
            reading("s5", "erin", 15, 150.0, SensorClass::Subghz, None, None, true, "h5"),
        ]
    }

    #[test]
    fn save_then_load_round_trips_groups_and_dedup_set() {
        let path = temp_state_path("roundtrip");
        let _guard = TempFileGuard(path.clone());

        let mut idx = AggregationIndex::new(120);
        idx.ingest_batch(&batch_a());
        idx.save_to_file(&path).expect("save should succeed");

        let reloaded = AggregationIndex::load_from_file(&path)
            .expect("load should succeed")
            .expect("file should exist and parse");

        // Groups/stats match.
        let before = idx.query(&QueryFilter::default());
        let after = reloaded.query(&QueryFilter::default());
        assert_eq!(before.len(), after.len());
        for b in &before {
            let a = after.iter().find(|g| g.key == b.key).expect("group missing after reload");
            assert_eq!(a.count, b.count);
            assert_eq!(a.verified_count, b.verified_count);
            assert_eq!(a.unverified_count, b.unverified_count);
            assert_eq!(a.stats, b.stats);
        }

        // Index-level counters match.
        let before_stats = idx.stats();
        let after_stats = reloaded.stats();
        assert_eq!(before_stats.total_ingested, after_stats.total_ingested);
        assert_eq!(before_stats.total_duplicates_skipped, after_stats.total_duplicates_skipped);

        // Dedup set was reconstructed: re-ingesting an already-seen reading
        // into the reloaded index must be rejected as a duplicate.
        let mut reloaded = reloaded;
        let dup = reading("s1", "alice", 10, 1.0, SensorClass::Gnss, Some(1.0), Some(1.0), true, "h1");
        assert!(!reloaded.ingest(&dup), "reloaded index must recognize pre-restart reading as a duplicate");
    }

    #[test]
    fn load_from_file_returns_none_when_no_file_exists_yet() {
        let path = temp_state_path("missing");
        // Deliberately do not create the file.
        let result = AggregationIndex::load_from_file(&path).expect("missing file is not an error");
        assert!(result.is_none());
    }

    /// The core self-heal scenario: ingest a batch, persist, simulate a
    /// restart (drop the in-memory index entirely, reconstruct fresh from
    /// disk), then re-ingest a batch that overlaps with what was already
    /// seen pre-restart. Assert:
    ///   (a) no previously-ingested reading is lost, and
    ///   (b) no reading is double-counted — final group stats/counts must be
    ///       identical to a single no-restart run over the *union* of
    ///       readings (batch_a plus the two genuinely-new readings from
    ///       batch_b).
    #[test]
    fn restart_does_not_lose_or_double_count_inflight_readings() {
        let path = temp_state_path("restart");
        let _guard = TempFileGuard(path.clone());

        // --- Pre-restart process ---
        let mut idx = AggregationIndex::new(120);
        let (newly_a, dupes_a) = idx.ingest_batch(&batch_a());
        assert_eq!(newly_a, 3);
        assert_eq!(dupes_a, 0);
        idx.save_to_file(&path).expect("persist before restart");

        // --- Simulated restart: drop the in-memory index entirely. ---
        drop(idx);
        let mut idx = AggregationIndex::load_from_file(&path)
            .expect("load after restart should succeed")
            .expect("snapshot file must exist")
        ;

        // --- Post-restart process re-ingests an overlapping+new batch, as
        // a crash-and-replay ingest source would. ---
        let (newly_b, dupes_b) = idx.ingest_batch(&batch_b_overlapping_and_new());
        assert_eq!(newly_b, 2, "only the 2 genuinely-new readings should be newly counted");
        assert_eq!(dupes_b, 2, "the 2 replayed pre-restart readings must be recognized as duplicates");

        // (a) Nothing lost: every reading from batch_a is still represented.
        let final_groups = idx.query(&QueryFilter::default());
        let gnss = final_groups.iter().find(|g| g.key.class == "gnss").expect("gnss group missing");
        let subghz = final_groups.iter().find(|g| g.key.class == "subghz").expect("subghz group missing");

        // gnss: s1 (batch_a), s2 (batch_a), s4 (batch_b, new) = 3 readings.
        assert_eq!(gnss.count, 3);
        assert!(gnss.provenance.iter().any(|p| p.sensor_id == "s1"), "s1 (pre-restart) lost");
        assert!(gnss.provenance.iter().any(|p| p.sensor_id == "s2"), "s2 (pre-restart) lost");
        assert!(gnss.provenance.iter().any(|p| p.sensor_id == "s4"), "s4 (post-restart new) missing");

        // subghz: s3 (batch_a), s5 (batch_b, new) = 2 readings. s3 must NOT
        // appear twice even though it was replayed.
        assert_eq!(subghz.count, 2);
        let s3_occurrences = subghz.provenance.iter().filter(|p| p.sensor_id == "s3").count();
        assert_eq!(s3_occurrences, 1, "s3 must not be double-counted after restart replay");
        assert!(subghz.provenance.iter().any(|p| p.sensor_id == "s5"), "s5 (post-restart new) missing");

        // (b) No double-counting: compare against a no-restart run ingesting
        // the union of readings (batch_a + the 2 new-only readings from
        // batch_b) in one go — group stats/counts must be identical.
        let mut no_restart = AggregationIndex::new(120);
        let mut union = batch_a();
        union.push(reading("s4", "dave", 20, 3.0, SensorClass::Gnss, Some(1.0), Some(1.0), true, "h4"));
        union.push(reading("s5", "erin", 15, 150.0, SensorClass::Subghz, None, None, true, "h5"));
        no_restart.ingest_batch(&union);

        let restarted_groups = idx.query(&QueryFilter::default());
        let baseline_groups = no_restart.query(&QueryFilter::default());
        assert_eq!(restarted_groups.len(), baseline_groups.len());
        for rg in &restarted_groups {
            let bg = baseline_groups.iter().find(|g| g.key == rg.key).expect("group key mismatch vs baseline");
            assert_eq!(rg.count, bg.count, "count mismatch for group {:?}", rg.key);
            assert_eq!(rg.verified_count, bg.verified_count);
            assert_eq!(rg.unverified_count, bg.unverified_count);
            assert_eq!(rg.stats, bg.stats, "stats mismatch for group {:?}", rg.key);
        }

        let restarted_stats = idx.stats();
        let baseline_stats = no_restart.stats();
        assert_eq!(restarted_stats.total_ingested, baseline_stats.total_ingested);
    }

    /// Same restart scenario but exercised through `AppState::with_persistence`
    /// (the actual production wiring in `state.rs`/`main.rs`), rather than
    /// calling `AggregationIndex::save_to_file`/`load_from_file` directly —
    /// covers the code path a real process restart takes.
    #[test]
    fn app_state_with_persistence_survives_restart_via_ingest_items() {
        use crate::ingest::IngestItem;
        use crate::state::AppState;
        use btcpc_types::LedgerEntry;

        let path = temp_state_path("appstate-restart");
        let _guard = TempFileGuard(path.clone());

        fn entry(sensor_id: &str, owner: &str, epoch: u64, value: f64, class: &str, data_hash: &str) -> LedgerEntry {
            LedgerEntry::SensorReading {
                sensor_id: sensor_id.to_string(),
                owner: owner.to_string(),
                epoch,
                value,
                data_hash: data_hash.to_string(),
                metadata: Some(serde_json::json!({
                    "class": class,
                    "schema": "1",
                    "payload": {}
                })),
                signed_by: owner.to_string(),
            }
        }

        // --- Pre-restart process ---
        let state1 = AppState::with_persistence(120, path.clone()).expect("fresh state should init");
        let items_pre = vec![
            IngestItem::chain_confirmed(entry("s1", "alice", 10, 1.0, "gnss", "h1")),
            IngestItem::chain_confirmed(entry("s2", "bob", 10, 2.0, "gnss", "h2")),
        ];
        let (newly, dupes, rejected) = state1.ingest_items(&items_pre);
        assert_eq!((newly, dupes, rejected), (2, 0, 0));
        drop(state1); // simulate process exit

        // --- Restart: reconstruct AppState from the same path. ---
        let state2 = AppState::with_persistence(120, path.clone()).expect("reload should succeed");
        let items_post = vec![
            // Replay of s1 (already seen pre-restart) plus one new reading.
            IngestItem::chain_confirmed(entry("s1", "alice", 10, 1.0, "gnss", "h1")),
            IngestItem::chain_confirmed(entry("s3", "carol", 10, 3.0, "gnss", "h3")),
        ];
        let (newly2, dupes2, rejected2) = state2.ingest_items(&items_post);
        assert_eq!(newly2, 1, "only s3 should be newly counted post-restart");
        assert_eq!(dupes2, 1, "replayed s1 must be recognized as a duplicate");
        assert_eq!(rejected2, 0);

        let groups = state2.index.read().query(&QueryFilter::default());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].count, 3, "s1, s2, s3 -> 3 total, no loss, no double-count");
    }
}

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
#[derive(Debug, Clone, Default)]
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
}

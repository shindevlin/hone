//! Shared, thread-safe application state wrapping the pure `AggregationIndex`.
//!
//! v2: optionally backed by an on-disk snapshot (see `agg::AggregationIndex::
//! save_to_file` / `load_from_file`) so a process restart does not lose or
//! double-count in-flight readings (Phase 1.2 self-heal requirement). This is
//! purely additive — `AppState::new`/`default_window` (no persistence path)
//! keep working exactly as before, and the HTTP query/ingest contract in
//! `api.rs` is untouched either way.

use crate::agg::AggregationIndex;
use crate::ingest::IngestItem;
use crate::model::DEFAULT_WINDOW_EPOCHS;
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub index: Arc<RwLock<AggregationIndex>>,
    /// Where to persist the index on `persist()`, if persistence is enabled
    /// for this `AppState`. `None` means in-memory-only (e.g. most existing
    /// unit/API tests), matching v1 behavior exactly.
    persist_path: Option<PathBuf>,
}

impl AppState {
    pub fn new(window_epochs: u64) -> Self {
        AppState {
            index: Arc::new(RwLock::new(AggregationIndex::new(window_epochs))),
            persist_path: None,
        }
    }

    pub fn default_window() -> Self {
        Self::new(DEFAULT_WINDOW_EPOCHS)
    }

    /// Build state backed by a persistence file: loads any existing snapshot
    /// at `path` (reconstructing groups + dedup set), or starts fresh if
    /// none exists yet. Every successful `ingest_items` call after this
    /// persists the updated index back to `path`.
    pub fn with_persistence(window_epochs: u64, path: PathBuf) -> std::io::Result<Self> {
        let index = match AggregationIndex::load_from_file(&path)? {
            Some(loaded) => loaded,
            None => AggregationIndex::new(window_epochs),
        };
        Ok(AppState {
            index: Arc::new(RwLock::new(index)),
            persist_path: Some(path),
        })
    }

    /// Write the current index state to `persist_path`, if persistence is
    /// enabled. No-op (Ok) if persistence isn't configured.
    pub fn persist(&self) -> std::io::Result<()> {
        if let Some(path) = &self.persist_path {
            self.index.read().save_to_file(path)?;
        }
        Ok(())
    }

    /// Ingest a batch of raw entries (with explicit trust source), returning
    /// (newly_counted, duplicates_skipped, rejected). If persistence is
    /// configured, the updated index is durably saved before returning, so a
    /// crash immediately after a successful ingest response does not lose
    /// that batch.
    pub fn ingest_items(&self, items: &[IngestItem]) -> (u64, u64, u64) {
        let batch = crate::ingest::parse_batch(items);
        let (newly, dupes) = {
            let mut idx = self.index.write();
            let (newly, dupes) = idx.ingest_batch(&batch.parsed);
            idx.record_rejected(batch.errors.len() as u64);
            (newly, dupes)
        };
        if let Err(e) = self.persist() {
            tracing::warn!("verasens: failed to persist aggregation index: {e}");
        }
        (newly, dupes, batch.errors.len() as u64)
    }
}

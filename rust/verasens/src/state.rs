//! Shared, thread-safe application state wrapping the pure `AggregationIndex`.

use crate::agg::AggregationIndex;
use crate::ingest::IngestItem;
use crate::model::DEFAULT_WINDOW_EPOCHS;
use parking_lot::RwLock;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub index: Arc<RwLock<AggregationIndex>>,
}

impl AppState {
    pub fn new(window_epochs: u64) -> Self {
        AppState {
            index: Arc::new(RwLock::new(AggregationIndex::new(window_epochs))),
        }
    }

    pub fn default_window() -> Self {
        Self::new(DEFAULT_WINDOW_EPOCHS)
    }

    /// Ingest a batch of raw entries (with explicit trust source), returning
    /// (newly_counted, duplicates_skipped, rejected).
    pub fn ingest_items(&self, items: &[IngestItem]) -> (u64, u64, u64) {
        let batch = crate::ingest::parse_batch(items);
        let mut idx = self.index.write();
        let (newly, dupes) = idx.ingest_batch(&batch.parsed);
        idx.record_rejected(batch.errors.len() as u64);
        (newly, dupes, batch.errors.len() as u64)
    }
}

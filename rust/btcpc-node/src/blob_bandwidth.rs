//! Blob bandwidth metering — in-memory per-CID byte counter, periodically flushed.
//! P5-F: ~53 LOC

#![allow(dead_code)]
use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::Mutex;

#[derive(Default)]
pub struct BlobBandwidthMeter {
    /// cid → bytes served since last flush
    counters: Arc<Mutex<HashMap<String, u64>>>,
}

impl BlobBandwidthMeter {
    pub fn new() -> Self { Self::default() }

    /// Record bytes served for a CID.
    pub fn record(&self, cid: &str, bytes: u64) {
        *self.counters.lock().entry(cid.to_string()).or_default() += bytes;
    }

    /// Flush and return current counts, resetting all counters.
    pub fn flush(&self) -> HashMap<String, u64> {
        let mut lock = self.counters.lock();
        std::mem::take(&mut *lock)
    }

    /// Peek at current counter for a CID without flushing.
    pub fn peek(&self, cid: &str) -> u64 {
        *self.counters.lock().get(cid).unwrap_or(&0)
    }
}

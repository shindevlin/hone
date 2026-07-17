//! Durable intent store.
//!
//! The store is the bridge's crash-safety substrate: every status transition
//! is persisted BEFORE the corresponding side effect (nonce pinned before
//! submit, `posted` written only after a confirmed seal), so a crash at any
//! point leaves the intent in a state `redrive()` can recover from.
//!
//! The trait is `IntentStore`; two impls are provided:
//! - `JsonFileStore` — self-contained, fsync-on-write, crash-tested. Chosen so
//!   this crate builds and tests standalone without a Postgres dependency.
//! - `MemStore` — in-memory, for fast unit tests.
//!
//! Swapping in the B2B service's Postgres `settlement_intent` table is a
//! trait-impl change, not a logic change (the worker only talks to
//! `IntentStore`).
//!
//! The store enforces the application-level idempotency UNIQUE constraint on
//! `(sensor_id, batch_hash, usage_event_id)`: `enqueue` of a duplicate is
//! rejected, so a retried query can never create a second intent.

use crate::intent::{IntentStatus, SettlementIntent};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("duplicate intent for idempotency key {0:?}")]
    Duplicate((String, String, String)),
    #[error("intent {0} not found")]
    NotFound(String),
    #[error("io error: {0}")]
    Io(String),
    #[error("serialization error: {0}")]
    Serde(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

/// Durable store of settlement intents. All methods are synchronous and must
/// be crash-safe: a returned `Ok` means the change is durable.
pub trait IntentStore {
    /// Enqueue a fresh intent. Rejects a duplicate idempotency key
    /// `(sensor_id, batch_hash, usage_event_id)` with `Duplicate`.
    fn enqueue(&mut self, intent: SettlementIntent) -> StoreResult<()>;

    /// Persist an updated intent (overwrites by `id`). Durable on return.
    fn update(&mut self, intent: &SettlementIntent) -> StoreResult<()>;

    /// Fetch by id.
    fn get(&self, id: &str) -> StoreResult<Option<SettlementIntent>>;

    /// All intents that are still open (not `Posted`), i.e. the redrive work
    /// set: `Pending | Posting | Failed`. Deterministically ordered by
    /// `(created_at, id)`. Quarantined intents are INCLUDED here (the worker
    /// decides to skip them) so a reconciliation report can see them.
    fn open_intents(&self) -> StoreResult<Vec<SettlementIntent>>;

    /// All intents (for reconciliation reporting).
    fn all(&self) -> StoreResult<Vec<SettlementIntent>>;
}

/// In-memory store for fast unit tests.
#[derive(Debug, Default)]
pub struct MemStore {
    by_id: HashMap<String, SettlementIntent>,
    keys: std::collections::HashSet<(String, String, String)>,
}

impl MemStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl IntentStore for MemStore {
    fn enqueue(&mut self, intent: SettlementIntent) -> StoreResult<()> {
        let key = intent.idempotency_key();
        if self.keys.contains(&key) {
            return Err(StoreError::Duplicate(key));
        }
        self.keys.insert(key);
        self.by_id.insert(intent.id.clone(), intent);
        Ok(())
    }
    fn update(&mut self, intent: &SettlementIntent) -> StoreResult<()> {
        if !self.by_id.contains_key(&intent.id) {
            return Err(StoreError::NotFound(intent.id.clone()));
        }
        self.by_id.insert(intent.id.clone(), intent.clone());
        Ok(())
    }
    fn get(&self, id: &str) -> StoreResult<Option<SettlementIntent>> {
        Ok(self.by_id.get(id).cloned())
    }
    fn open_intents(&self) -> StoreResult<Vec<SettlementIntent>> {
        let mut v: Vec<_> = self
            .by_id
            .values()
            .filter(|i| i.status.is_open())
            .cloned()
            .collect();
        v.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        Ok(v)
    }
    fn all(&self) -> StoreResult<Vec<SettlementIntent>> {
        let mut v: Vec<_> = self.by_id.values().cloned().collect();
        v.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        Ok(v)
    }
}

/// Crash-safe JSON-file-backed store. The entire intent set is serialized to a
/// single file; each mutation writes to a temp file, fsyncs it, then atomically
/// renames over the target (so a crash mid-write never corrupts the store — the
/// old file survives intact until the rename commits). On construction it loads
/// any existing file, so a restart sees every previously-persisted intent — the
/// substrate that makes `redrive()` self-healing across process restarts.
#[derive(Debug)]
pub struct JsonFileStore {
    path: std::path::PathBuf,
    mem: MemStore,
}

impl JsonFileStore {
    /// Open (or create) a store at `path`, loading any existing intents.
    pub fn open(path: impl Into<std::path::PathBuf>) -> StoreResult<Self> {
        let path = path.into();
        let mut mem = MemStore::new();
        if path.exists() {
            let bytes = std::fs::read(&path).map_err(|e| StoreError::Io(e.to_string()))?;
            if !bytes.is_empty() {
                let intents: Vec<SettlementIntent> = serde_json::from_slice(&bytes)
                    .map_err(|e| StoreError::Serde(e.to_string()))?;
                for i in intents {
                    // Load directly (bypass duplicate check — persisted set is
                    // already deduplicated by construction).
                    mem.keys.insert(i.idempotency_key());
                    mem.by_id.insert(i.id.clone(), i);
                }
            }
        }
        Ok(JsonFileStore { path, mem })
    }

    fn flush(&self) -> StoreResult<()> {
        let all = self.mem.all()?;
        let bytes = serde_json::to_vec_pretty(&all).map_err(|e| StoreError::Serde(e.to_string()))?;
        // Write to temp, fsync, atomic rename.
        let tmp = self.path.with_extension("tmp");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp).map_err(|e| StoreError::Io(e.to_string()))?;
            f.write_all(&bytes).map_err(|e| StoreError::Io(e.to_string()))?;
            f.sync_all().map_err(|e| StoreError::Io(e.to_string()))?;
        }
        std::fs::rename(&tmp, &self.path).map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(())
    }
}

impl IntentStore for JsonFileStore {
    fn enqueue(&mut self, intent: SettlementIntent) -> StoreResult<()> {
        self.mem.enqueue(intent)?;
        self.flush()
    }
    fn update(&mut self, intent: &SettlementIntent) -> StoreResult<()> {
        self.mem.update(intent)?;
        self.flush()
    }
    fn get(&self, id: &str) -> StoreResult<Option<SettlementIntent>> {
        self.mem.get(id)
    }
    fn open_intents(&self) -> StoreResult<Vec<SettlementIntent>> {
        self.mem.open_intents()
    }
    fn all(&self) -> StoreResult<Vec<SettlementIntent>> {
        self.mem.all()
    }
}

/// Reconciliation totals for the internal report the seam requires: `Σ hunits
/// posted for owners` must correspond to `Σ billable USD` at the pinned rates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reconciliation {
    pub total_intents: usize,
    pub posted: usize,
    pub open: usize,
    pub quarantined: usize,
    pub usd_cents_posted: u64,
    pub hunits_posted: u64,
    pub usd_cents_open: u64,
}

/// Compute reconciliation totals from a store snapshot.
pub fn reconcile(store: &dyn IntentStore) -> StoreResult<Reconciliation> {
    let all = store.all()?;
    let mut r = Reconciliation {
        total_intents: all.len(),
        posted: 0,
        open: 0,
        quarantined: 0,
        usd_cents_posted: 0,
        hunits_posted: 0,
        usd_cents_open: 0,
    };
    for i in &all {
        match i.status {
            IntentStatus::Posted => {
                r.posted += 1;
                r.usd_cents_posted = r.usd_cents_posted.saturating_add(i.usd_amount_cents);
                r.hunits_posted = r.hunits_posted.saturating_add(i.dreams_amount.unwrap_or(0));
            }
            _ => {
                r.open += 1;
                r.usd_cents_open = r.usd_cents_open.saturating_add(i.usd_amount_cents);
                if i.quarantined {
                    r.quarantined += 1;
                }
            }
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intent::FxRateId;

    fn mk(id: &str, ueid: &str, sensor: &str, batch: &str) -> SettlementIntent {
        SettlementIntent::new_pending(
            id,
            ueid,
            sensor,
            "owner-acct",
            batch,
            100,
            FxRateId("r1".into()),
            42,
        )
    }

    #[test]
    fn mem_rejects_duplicate_idempotency_key() {
        let mut s = MemStore::new();
        s.enqueue(mk("i1", "q1", "s1", "b1")).unwrap();
        // Same (sensor, batch, usage_event) => duplicate, even with a new id.
        let err = s.enqueue(mk("i2", "q1", "s1", "b1")).unwrap_err();
        assert!(matches!(err, StoreError::Duplicate(_)));
    }

    #[test]
    fn mem_allows_same_sensor_batch_for_different_query() {
        let mut s = MemStore::new();
        s.enqueue(mk("i1", "q1", "s1", "b1")).unwrap();
        // Different usage_event_id => a genuine second sale, allowed.
        s.enqueue(mk("i2", "q2", "s1", "b1")).unwrap();
        assert_eq!(s.all().unwrap().len(), 2);
    }

    #[test]
    fn json_file_store_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("intents.json");
        {
            let mut s = JsonFileStore::open(&path).unwrap();
            s.enqueue(mk("i1", "q1", "s1", "b1")).unwrap();
            s.enqueue(mk("i2", "q2", "s2", "b2")).unwrap();
        }
        // Reopen: both intents must be present (this is the restart substrate).
        let s2 = JsonFileStore::open(&path).unwrap();
        assert_eq!(s2.open_intents().unwrap().len(), 2);
        assert!(s2.get("i1").unwrap().is_some());
    }

    #[test]
    fn json_file_store_persists_duplicate_guard_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("intents.json");
        {
            let mut s = JsonFileStore::open(&path).unwrap();
            s.enqueue(mk("i1", "q1", "s1", "b1")).unwrap();
        }
        let mut s2 = JsonFileStore::open(&path).unwrap();
        // Duplicate guard must survive a restart.
        let err = s2.enqueue(mk("i9", "q1", "s1", "b1")).unwrap_err();
        assert!(matches!(err, StoreError::Duplicate(_)));
    }
}

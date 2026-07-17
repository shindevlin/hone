//! The settlement intent — the bridge's unit of work.
//!
//! A billable USD query enqueues one `SettlementIntent` per contributing
//! `(sensor_id, batch_hash)` in the SAME DB transaction that records the
//! usage event (seam requirement, enforced by the caller / dashboard layer,
//! not by this crate). The bridge drains these to `Posted` by posting a
//! `SensorDataPurchase` on-chain.
//!
//! See the PLATFORM_PRD.md 1.4 settlement-bridge DECISION RECORD for the
//! binding design this implements.

use serde::{Deserialize, Serialize};

/// Lifecycle of a settlement intent. The ONLY terminal-success state is
/// `Posted` — the owner has actually been paid on-chain. `Failed` is NOT
/// terminal for a well-formed intent: it means "retry me on the next
/// redrive". A malformed intent is `Failed` with `last_error` set and
/// `quarantined = true`, which stops the retry loop and surfaces it to a
/// human (never silently dropped — that would be "collected but never
/// settled").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    /// Enqueued, not yet attempted.
    Pending,
    /// Nonce + epoch pinned, entry submitted (or about to be). A crash here
    /// is the dangerous case: we may or may not have hit the chain. Redrive
    /// resolves it by querying the chain for the pinned entry before
    /// re-submitting.
    Posting,
    /// Owner paid on-chain, `chain_entry_hash` recorded. Never re-touched.
    Posted,
    /// Retryable failure (no peers, timeout, insufficient balance, stale fx).
    /// Redrive retries. If `quarantined`, redrive skips it (malformed).
    Failed,
}

impl IntentStatus {
    /// Is this intent still work for the bridge? `Posted` is done; everything
    /// else is re-driven on the next tick.
    pub fn is_open(&self) -> bool {
        !matches!(self, IntentStatus::Posted)
    }
}

/// Reference to the pinned USD->HONE fx rate used to convert this intent's
/// `usd_amount_cents` into `dreams_amount`. Captured at intent creation so the
/// conversion is auditable and immune to a rate that moves between metering
/// and posting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FxRateId(pub String);

/// The bridge's unit of work. Field set matches the 1.4 seam object plus the
/// crash-safety columns (`nonce`, `epoch`, `attempts`, `last_error`,
/// `quarantined`) documented in the DECISION RECORD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementIntent {
    /// Stable unique id of this intent (assigned by the enqueuing layer).
    pub id: String,
    /// The usage_event this intent settles. Part of the idempotency key so a
    /// retried *query* cannot create a second intent, but the same sensor+batch
    /// bought by a different query settles again (a genuine second sale).
    pub usage_event_id: String,
    /// Sensor whose data was queried.
    pub sensor_id: String,
    /// Owner account to be paid. THE load-bearing economic field: the chain
    /// pays this account. Must come only from verified aggregator provenance.
    pub owner_account: String,
    /// Batch discriminator. The aggregator supplies the reading `data_hash`
    /// (or a real `SensorDataCommit.batch_hash` once Phase 1.2 can). Goes into
    /// the on-chain entry's `batch_hash` field as an audit label; the chain
    /// does not validate it, so it is truthful provenance, not a guess.
    pub batch_hash: String,
    /// USD charged for the query slice this intent settles, in cents.
    pub usd_amount_cents: u64,
    /// Resolved HONE payout in hunits (1 HONE = 10_000_000_000 hunits). `None`
    /// until first post attempt resolves it from `usd_amount_cents` via the
    /// pinned fx rate.
    pub dreams_amount: Option<u64>,
    /// Pinned fx rate reference. The bridge resolves `dreams_amount` ONLY
    /// through this — it never fetches a live rate at post time.
    pub fx_rate_id: FxRateId,
    /// Lifecycle.
    pub status: IntentStatus,
    /// On-chain entry hash once posted.
    pub chain_entry_hash: Option<String>,
    /// Settlement-account nonce pinned at first post attempt. A retry re-posts
    /// the SAME entry with this nonce; the chain accepts a given nonce at most
    /// once, so a retry cannot double-post.
    pub nonce: Option<u64>,
    /// Chain epoch captured at first post attempt. Pinned because the canonical
    /// signing message for SensorDataPurchase includes the full entry (epoch
    /// included); a retry must re-sign the identical bytes.
    pub epoch: Option<u64>,
    /// Retry bookkeeping.
    pub attempts: u32,
    /// Last error observed (for the reconciliation report).
    pub last_error: Option<String>,
    /// If true, redrive skips this intent (malformed — needs a human).
    pub quarantined: bool,
    pub created_at: u64,
    pub posted_at: Option<u64>,
}

impl SettlementIntent {
    /// The application-level idempotency key: `(sensor_id, batch_hash,
    /// usage_event_id)`. The store enforces this UNIQUE so a duplicate intent
    /// can never be enqueued.
    pub fn idempotency_key(&self) -> (String, String, String) {
        (
            self.sensor_id.clone(),
            self.batch_hash.clone(),
            self.usage_event_id.clone(),
        )
    }

    /// Construct a fresh `Pending` intent. `dreams_amount`/`nonce`/`epoch` are
    /// resolved later at post time.
    #[allow(clippy::too_many_arguments)]
    pub fn new_pending(
        id: impl Into<String>,
        usage_event_id: impl Into<String>,
        sensor_id: impl Into<String>,
        owner_account: impl Into<String>,
        batch_hash: impl Into<String>,
        usd_amount_cents: u64,
        fx_rate_id: FxRateId,
        created_at: u64,
    ) -> Self {
        SettlementIntent {
            id: id.into(),
            usage_event_id: usage_event_id.into(),
            sensor_id: sensor_id.into(),
            owner_account: owner_account.into(),
            batch_hash: batch_hash.into(),
            usd_amount_cents,
            dreams_amount: None,
            fx_rate_id,
            status: IntentStatus::Pending,
            chain_entry_hash: None,
            nonce: None,
            epoch: None,
            attempts: 0,
            last_error: None,
            quarantined: false,
            created_at,
            posted_at: None,
        }
    }
}

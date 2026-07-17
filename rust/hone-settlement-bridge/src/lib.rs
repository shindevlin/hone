//! # hone-settlement-bridge
//!
//! USD -> HONE settlement bridge. When an institutional buyer pays USD for a
//! Verasens query, the underlying `SensorDataPurchase` fee must still be posted
//! on-chain in HONE/hunits so the sensor owner is paid in the native token,
//! regardless of how the buyer paid. This crate is the crash-safe, self-healing
//! worker that guarantees "a USD payment can never be collected but never
//! settled".
//!
//! The binding design is the DECISION RECORD in `docs/PLATFORM_PRD.md` under
//! Phase 1.4's "USD -> HONE settlement bridge" item. In particular:
//!
//! - **Provenance-gap resolution.** The Phase 1.2 aggregator exposes provenance
//!   as `(sensor_id, owner, data_hash, verified)` — no `batch_hash`. Because the
//!   chain pays the `owner` field and IGNORES `batch_hash` (verified in
//!   `chain.rs`), the bridge settles per-`(sensor_id, owner)` and carries the
//!   aggregator's `data_hash` as the on-chain `batch_hash` audit label. This is
//!   the correct money to the correct owner with a truthful label.
//! - **Idempotency.** App-level UNIQUE `(sensor_id, batch_hash, usage_event_id)`
//!   stops duplicate intents; the pinned settlement-account nonce stops
//!   duplicate posts (the chain accepts a nonce at most once).
//! - **Crash recovery.** `redrive()` re-drives every non-`Posted` intent on
//!   startup and each tick; the durable store is the substrate.
//! - **FX pinning.** The USD->HONE rate is pinned at intent creation, never
//!   re-derived at post time; stale / out-of-band / missing rates are rejected
//!   (never posted at a guessed rate).
//!
//! ## Real vs. mocked
//!
//! Everything except the chain submission call is real. The chain call is the
//! `ChainSubmit` trait; production wires it to `apply_and_broadcast` in
//! `rust/hone-node/src/api.rs`. A documented pre-existing rustc ICE blocks
//! building the full `hone-node` binary here, so tests exercise the bridge
//! against `MockChain` (which models the chain's real nonce / no-peers /
//! balance semantics). No test claims a real on-chain post succeeded.

pub mod chain;
pub mod fx;
pub mod intent;
pub mod store;
pub mod worker;

pub use chain::{
    build_purchase_entry, canonical_signing_message, sign_entry, ChainSubmit, MockChain,
    SigningIdentity, SubmitOutcome,
};
pub use fx::{FxError, FxPolicy, FxRate, FxRateBook};
pub use intent::{FxRateId, IntentStatus, SettlementIntent};
pub use store::{reconcile, IntentStore, JsonFileStore, MemStore, Reconciliation, StoreError};
pub use worker::{DriveResult, SettlementWorker, WorkerConfig};

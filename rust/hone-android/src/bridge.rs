//! uniffi bridge — the typed Kotlin ⇆ Rust interface for the HONE Android app.
//!
//! This REPLACES the hand-written JNI in `lib.rs`. Instead of ~10 flat
//! `extern "C"` functions with manual string marshalling and `unsafe`, the app
//! sees:
//!   * `HoneNode`   — an object you construct once and call methods on
//!   * `NodeConfig` — a typed record (no positional-arg drift)
//!   * `NodeError`  — a typed error enum (Kotlin gets real exceptions)
//!   * `NodeStatus` — a typed snapshot for the UI
//!
//! uniffi generates the matching Kotlin (`HoneNode`, `NodeConfig`, sealed
//! `NodeException`, `NodeStatus`) so a signature change in Rust is a COMPILE
//! error in Kotlin, not a runtime `UnsatisfiedLinkError`. See
//! docs/ANDROID_WORLDCLASS_PLAN.md §3.
//!
//! Design notes:
//!   * Miner-node and clock-node were two separate global cells in the JNI. Here
//!     each `HoneNode` instance owns its own runtime + handle, so the app can
//!     hold one for the miner role and one for the clock role (or a single
//!     combined node) without global statics. Cleaner and testable.
//!   * All blocking work runs on the node's own tokio runtime; the exported
//!     methods are cheap and non-async so Kotlin can call them from a service
//!     without an executor. (Long operations already run inside the node.)

use std::sync::Arc;
use parking_lot::Mutex as PLMutex;

use crate::node::{self, NodeConfig as CoreConfig, NodeHandle};
use crate::sensors;

// NOTE: `uniffi::setup_scaffolding!()` MUST be called at the crate root (lib.rs),
// not here — it generates a crate-root `UniFfiTag` type the derives reference.

// ─────────────────────────────────────────────────────────────────────────────
// Public typed surface
// ─────────────────────────────────────────────────────────────────────────────

/// Configuration to start a node. Mirrors `node::NodeConfig` but is the
/// uniffi-exported, Kotlin-visible record. Kept 1:1 so the mapping is obvious.
#[derive(uniffi::Record, Clone)]
pub struct NodeConfig {
    pub account: String,
    pub posting_key: String,
    pub chain_id: String,
    pub genesis_ts: u64,
    pub data_dir: String,
    /// On-device GGUF model id (empty = no local inference for this node).
    pub model_id: String,
    pub model_dir: String,
    /// Comma-free: the app passes a real list, not a CSV string.
    pub bootstrap_peers: Vec<String>,
    pub p2p_port: u16,
    pub is_miner: bool,
    pub is_clock: bool,
}

/// A snapshot of node state for the UI. One typed call instead of four
/// (`getStatus`/`getEpoch`/`getBalance`/`isRunning`) — fewer round-trips, and
/// the values are guaranteed consistent (read under one lock).
#[derive(uniffi::Record, Clone)]
pub struct NodeStatus {
    pub running: bool,
    pub status_text: String,
    pub epoch: u64,
    /// Balance of the node's own account, in hunits.
    pub balance_hunits: u64,
}

/// Typed errors surfaced to Kotlin as a sealed `NodeException`.
#[derive(uniffi::Error, thiserror::Error, Debug)]
pub enum NodeError {
    #[error("node is already running")]
    AlreadyRunning,
    #[error("node is not running")]
    NotRunning,
    #[error("failed to start node: {msg}")]
    StartFailed { msg: String },
    #[error("runtime error: {msg}")]
    Runtime { msg: String },
}

// ─────────────────────────────────────────────────────────────────────────────
// The HoneNode object
// ─────────────────────────────────────────────────────────────────────────────

/// A running (or stoppable) HONE micronode. Construct with `HoneNode::new()`,
/// then `start(config)`. Hold the instance in the Android foreground service.
#[derive(uniffi::Object)]
pub struct HoneNode {
    inner: PLMutex<Option<Running>>,
}

struct Running {
    handle: NodeHandle,
    runtime: tokio::runtime::Runtime,
}

#[uniffi::export]
impl HoneNode {
    /// Create an idle node handle. Cheap; does no work until `start`.
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self { inner: PLMutex::new(None) })
    }

    /// Start the node with the given config. Blocks until the node's async
    /// bootstrap (store open, genesis, swarm spawn) completes, then returns.
    /// The node keeps running on its own runtime after this returns.
    pub fn start(&self, config: NodeConfig) -> Result<(), NodeError> {
        let mut guard = self.inner.lock();
        if guard.is_some() {
            return Err(NodeError::AlreadyRunning);
        }

        let core = CoreConfig {
            account: config.account,
            posting_key: config.posting_key,
            chain_id: config.chain_id,
            genesis_ts: config.genesis_ts,
            data_dir: config.data_dir,
            model_id: config.model_id,
            model_dir: config.model_dir,
            bootstrap_peers: config.bootstrap_peers,
            p2p_port: config.p2p_port,
            is_miner: config.is_miner,
            is_clock: config.is_clock,
        };

        let threads = if core.is_miner { 3 } else { 2 };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(threads)
            .enable_all()
            .thread_name("hone-node")
            .build()
            .map_err(|e| NodeError::StartFailed { msg: format!("tokio: {e}") })?;

        let status = Arc::new(PLMutex::new("Starting…".to_owned()));
        let status_clone = status.clone();
        let result = runtime.block_on(async move { node::start(core, status_clone).await });

        match result {
            Ok(handle) => {
                *guard = Some(Running { handle, runtime });
                Ok(())
            }
            Err(e) => Err(NodeError::StartFailed { msg: e.to_string() }),
        }
    }

    /// Stop the node and cancel all its tasks. Idempotent-ish: returns
    /// `NotRunning` if already stopped so the caller can react.
    pub fn stop(&self) -> Result<(), NodeError> {
        let mut guard = self.inner.lock();
        match guard.take() {
            Some(running) => {
                let _ = running.handle.shutdown_tx.send(());
                drop(running); // runtime drop cancels spawned tasks
                Ok(())
            }
            None => Err(NodeError::NotRunning),
        }
    }

    /// True if the node is currently running.
    pub fn is_running(&self) -> bool {
        self.inner.lock().is_some()
    }

    /// One consistent snapshot for the UI (running/status/epoch/balance).
    pub fn status(&self) -> NodeStatus {
        let guard = self.inner.lock();
        match guard.as_ref() {
            Some(r) => NodeStatus {
                running: true,
                status_text: r.handle.status.lock().clone(),
                epoch: r.handle.chain.current_epoch(),
                balance_hunits: r
                    .handle
                    .chain
                    .store
                    .get_balance(&r.handle.chain.node_id, hone_types::NATIVE_TOKEN),
            },
            None => NodeStatus {
                running: false,
                status_text: "Stopped".to_owned(),
                epoch: 0,
                balance_hunits: 0,
            },
        }
    }

    /// Balance of an arbitrary account (hunits). 0 if not running / unknown.
    pub fn balance_of(&self, account: String) -> u64 {
        let guard = self.inner.lock();
        guard
            .as_ref()
            .map(|r| r.handle.chain.store.get_balance(&account, hone_types::NATIVE_TOKEN))
            .unwrap_or(0)
    }

    /// Submit a sensor reading. Returns `NotRunning` if the node is down.
    /// `values_json` is e.g. `{"x":1.2,"y":0.3,"z":9.8}` or `{"lat":..,"lon":..}`.
    pub fn submit_sensor_reading(
        &self,
        sensor_id: String,
        sensor_type: String,
        primary_value: f64,
        values_json: String,
        unit: String,
    ) -> Result<(), NodeError> {
        let guard = self.inner.lock();
        let running = guard.as_ref().ok_or(NodeError::NotRunning)?;

        let reading = sensors::SensorReading {
            sensor_id,
            sensor_type,
            primary_value,
            values_json,
            unit,
            owner: running.handle.chain.node_id.clone(),
            epoch: running.handle.chain.current_epoch(),
        };

        let chain = running.handle.chain.clone();
        let cmd_tx = running.handle.cmd_tx.clone();
        running.runtime.spawn(async move {
            sensors::submit(reading, chain, cmd_tx).await;
        });
        Ok(())
    }
}

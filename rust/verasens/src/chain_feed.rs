//! Thin adapter layer: polls a live `btcpc-node` HTTP API for `SensorReading`
//! entries and feeds them into an `AppState`'s aggregation index.
//!
//! This module is intentionally the *only* place that talks to a live node.
//! It does no aggregation logic itself — it just fetches, deserializes into
//! `RawSensorReading`, and calls into `state::AppState::ingest_items`, which
//! goes through the exact same `ingest::parse_batch` path as the HTTP
//! `/ingest` route and the unit tests. That's what keeps ingest testable
//! without a live chain: this adapter has no logic of its own worth testing
//! beyond "did it call the right things," and it's not exercised by the unit
//! test suite here (see main.rs doc comment / PRD notes on scope).
//!
//! v1 scope: polling only, not a gossip subscriber. The node is assumed to
//! expose sensor readings somewhere under its HTTP API (port 4242 by
//! convention — see root CLAUDE.md); the exact route is not yet finalized
//! upstream, so `node_sensor_readings_url` is configurable via env rather
//! than hardcoded, and this fetch is best-effort (log + skip on error, never
//! panic the service over a transient node hiccup).

use crate::ingest::IngestItem;
use crate::state::AppState;
use tracing::{info, warn};

pub struct ChainFeedConfig {
    /// Full URL to poll, e.g. `http://localhost:4242/api/sensor/readings`.
    /// Left as a plain URL (not host+port) since the exact node route is a
    /// live-chain integration detail outside this crate's control.
    pub url: String,
    pub poll_interval: std::time::Duration,
    /// Whether to trust each fetched reading's own `signed_by` field (i.e.
    /// treat this feed as chain-confirmed). Should be `true` once the node
    /// route in question only returns entries that passed
    /// `validate_and_apply`; kept configurable so a raw-mempool/gossip feed
    /// (not yet confirmed) can be wired in later as `false` without code
    /// changes here.
    pub trust_signed_by: bool,
}

/// Poll loop: fetches once per `poll_interval`, forever. Errors are logged
/// and swallowed — a node being temporarily unreachable must not crash the
/// aggregation service (self-heal posture per root CLAUDE.md / SELF_HEAL_PRD).
pub async fn run_poll_loop(state: AppState, cfg: ChainFeedConfig) {
    let client = reqwest::Client::new();
    let mut ticker = tokio::time::interval(cfg.poll_interval);
    loop {
        ticker.tick().await;
        if let Err(e) = poll_once(&client, &state, &cfg).await {
            warn!("verasens chain_feed poll failed (will retry next interval): {e}");
        }
    }
}

async fn poll_once(
    client: &reqwest::Client,
    state: &AppState,
    cfg: &ChainFeedConfig,
) -> anyhow::Result<()> {
    let resp = client.get(&cfg.url).send().await?.error_for_status()?;
    let raws: Vec<crate::ingest::RawSensorReading> = resp.json().await?;

    let items: Vec<IngestItem> = raws
        .into_iter()
        .map(|raw| {
            let entry = crate::ingest::raw_reading_to_entry(raw);
            if cfg.trust_signed_by {
                IngestItem::chain_confirmed(entry)
            } else {
                IngestItem::unconfirmed(entry)
            }
        })
        .collect();

    let (newly, dupes, rejected) = state.ingest_items(&items);
    info!(
        "verasens chain_feed poll: {} new, {} duplicate, {} rejected",
        newly, dupes, rejected
    );
    Ok(())
}

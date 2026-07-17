//! Verasens aggregation service.
//!
//! Ingests `SensorReading` events (via `/api/verasens/ingest`, or the
//! optional live-chain poller in `chain_feed.rs`), groups them by sensor
//! class + geography + time window (`agg::AggregationIndex`), and exposes a
//! query API that returns aggregated intelligence — counts, value stats,
//! verified/unverified breakdown, and provenance — never raw per-reading
//! dumps. See docs/PLATFORM_PRD.md Phase 1.2 for the product rationale and
//! the ratified `metadata` JSON convention this crate groups/queries against.

mod agg;
mod api;
mod chain_feed;
mod ingest;
mod model;
mod state;

use anyhow::Result;
use state::AppState;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("verasens=info".parse()?)
                .add_directive("tower_http=warn".parse()?),
        )
        .init();

    let port: u16 = std::env::var("VERASENS_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4343);

    let window_epochs: u64 = std::env::var("VERASENS_WINDOW_EPOCHS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(model::DEFAULT_WINDOW_EPOCHS);

    // Optional on-disk persistence: set VERASENS_STATE_PATH to survive
    // restarts without losing or double-counting in-flight readings (Phase
    // 1.2 self-heal requirement). Off by default (in-memory only, matching
    // v1) since not every deployment needs a durable state file.
    let state = match std::env::var("VERASENS_STATE_PATH") {
        Ok(path) => {
            let path = std::path::PathBuf::from(path);
            info!("Verasens persistence enabled: {}", path.display());
            AppState::with_persistence(window_epochs, path)?
        }
        Err(_) => {
            info!("Verasens persistence disabled (set VERASENS_STATE_PATH to enable)");
            if window_epochs == model::DEFAULT_WINDOW_EPOCHS {
                AppState::default_window()
            } else {
                AppState::new(window_epochs)
            }
        }
    };

    info!(
        "Verasens v{} starting — window = {} epochs ({}s)",
        env!("CARGO_PKG_VERSION"),
        window_epochs,
        window_epochs as f64 * btcpc_types::EPOCH_DURATION_S,
    );

    // Optional live-chain poll adapter. Off by default (v1 is designed to be
    // driven by POST /ingest, e.g. from a node-side forwarder or a backfill
    // script) — opt in with VERASENS_CHAIN_FEED_URL.
    if let Ok(url) = std::env::var("VERASENS_CHAIN_FEED_URL") {
        let poll_secs: u64 = std::env::var("VERASENS_CHAIN_FEED_POLL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);
        let trust_signed_by = std::env::var("VERASENS_CHAIN_FEED_TRUST_SIGNED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        info!("Verasens chain feed enabled: {url} (poll every {poll_secs}s, trust_signed_by={trust_signed_by})");
        let feed_state = state.clone();
        tokio::spawn(chain_feed::run_poll_loop(
            feed_state,
            chain_feed::ChainFeedConfig {
                url,
                poll_interval: std::time::Duration::from_secs(poll_secs),
                trust_signed_by,
            },
        ));
    } else {
        info!("Verasens chain feed disabled (set VERASENS_CHAIN_FEED_URL to enable polling a live node)");
    }

    let app = axum::Router::new()
        .nest("/api/verasens", api::router(state))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .layer(TraceLayer::new_for_http());

    let addr = format!("0.0.0.0:{port}");
    info!("Verasens listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

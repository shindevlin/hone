//! btcpc-ble-tracker — passive BLE tracker oracle for BTCPC sensor nodes.
//!
//! Runs on any Raspberry Pi (or Linux box) with a BLE adapter.
//! Optionally compile with `--features acoustic` for acoustic ownership challenges
//! (requires a USB microphone).
//!
//! Usage:
//!   btcpc-ble-tracker \
//!     --account shindevlin \
//!     --posting-key <hex> \
//!     --observer-id "pi-nebra/ble"
//!
//! Environment variables (all have --flag equivalents):
//!   BTCPC_NODE_URL, BTCPC_ACCOUNT, BTCPC_POSTING_KEY, BTCPC_OBSERVER_ID

mod api;
mod classify;
mod colocation;
mod commit;
mod config;
mod scanner;
mod subscription;

#[cfg(feature = "acoustic")]
mod acoustic;

use anyhow::Context;
use axum::{extract::State, routing::post, Json, Router};
use clap::Parser;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

use api::NodeClient;
use colocation::CoLocationTracker;
use commit::BatchCommitter;
use config::Config;
use subscription::SubscriptionManager;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("btcpc_ble_tracker=info".parse()?)
        )
        .init();

    let cfg = Config::parse();
    info!("btcpc-ble-tracker starting, observer_id={}", cfg.observer_id);

    let db = sled::open(&cfg.db_path)
        .with_context(|| format!("open sled DB at {}", cfg.db_path))?;

    let client = NodeClient::new(cfg.node_url.clone());

    // scanner → fanout task → (committer, subscription manager, acoustic bridge)
    let (scanner_tx, mut scanner_rx) = mpsc::channel::<scanner::Sighting>(256);
    let (committer_tx, committer_rx) = mpsc::channel::<scanner::Sighting>(256);
    let (sub_tx, mut sub_rx)         = mpsc::channel::<scanner::Sighting>(256);

    // ── BLE Scanner ───────────────────────────────────────────────────────────
    let min_rssi = cfg.min_rssi;
    tokio::spawn(async move {
        if let Err(e) = scanner::run(scanner_tx, min_rssi).await {
            tracing::error!("BLE scanner crashed: {}", e);
        }
    });

    // ── Fanout: one sighting → committer + subscription manager ──────────────
    tokio::spawn(async move {
        while let Some(s) = scanner_rx.recv().await {
            let _ = committer_tx.send(s.clone()).await;
            let _ = sub_tx.send(s).await;
        }
    });

    // ── Batch Committer ───────────────────────────────────────────────────────
    let committer = BatchCommitter::new(
        cfg.observer_id.clone(),
        cfg.account.clone(),
        cfg.epoch_secs,
        client.clone(),
        db.clone(),
    );
    tokio::spawn(async move {
        committer.run(committer_rx).await;
    });

    // ── Subscription push delivery ────────────────────────────────────────────
    let sub_manager = Arc::new(SubscriptionManager::new(
        client.clone(),
        cfg.observer_id.clone(),
        cfg.account.clone(),
        cfg.epoch_secs,
    ));
    {
        let sm = sub_manager.clone();
        tokio::spawn(async move { sm.run_sync().await; });
    }
    {
        let sm = sub_manager.clone();
        let epoch_secs = cfg.epoch_secs;
        tokio::spawn(async move {
            while let Some(sighting) = sub_rx.recv().await {
                let epoch = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    / epoch_secs;
                sm.handle_sighting(&sighting, epoch).await;
            }
        });
    }

    // ── Co-location Tracker (Option C) ───────────────────────────────────────
    let coloc = CoLocationTracker::new(
        db.clone(),
        client.clone(),
        cfg.observer_id.clone(),
        cfg.account.clone(),
        cfg.epoch_secs,
    );
    tokio::spawn(async move {
        coloc.run().await;
    });

    // ── Challenge API (Option A) ──────────────────────────────────────────────
    // Accepts POST /challenge/start from the node when a user initiates an
    // acoustic ownership challenge.
    #[cfg(feature = "acoustic")]
    {
        let cfg2 = cfg.clone();
        let client2 = client.clone();
        // Feed the challenge's co-event sightings back through the main scanner channel.
        // We create a fresh pair here so the challenge handler can inject sightings.
        let (ble_tx2, _ble_rx2) = mpsc::channel::<scanner::Sighting>(32);

        tokio::spawn(async move {
            let challenger = Arc::new(acoustic::AcousticChallenger::new(
                client2,
                cfg2.observer_id.clone(),
                cfg2.account.clone(),
                cfg2.epoch_secs,
            ));

            let app = Router::new()
                .route("/challenge/start", post(handle_challenge))
                .with_state(ChallengeState { challenger, ble_tx: ble_tx2 });

            let addr = format!("127.0.0.1:{}", cfg2.challenge_port);
            info!("acoustic challenge API on {}", addr);
            let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
            axum::serve(listener, app).await.unwrap();
        });
    }

    info!("all tasks running — Ctrl-C to stop");
    tokio::signal::ctrl_c().await?;
    info!("shutting down");
    Ok(())
}

#[cfg(feature = "acoustic")]
#[derive(Clone)]
struct ChallengeState {
    challenger: Arc<acoustic::AcousticChallenger>,
    ble_tx: mpsc::Sender<scanner::Sighting>,
}

#[cfg(feature = "acoustic")]
async fn handle_challenge(
    State(state): State<ChallengeState>,
    Json(req): Json<acoustic::ChallengeRequest>,
) -> Json<serde_json::Value> {
    let (tx, mut rx) = mpsc::channel::<scanner::Sighting>(32);

    // Bridge the main BLE channel into the challenge's receiver.
    let ble_tx = state.ble_tx.clone();
    let _bridge = tokio::spawn(async move {
        while let Some(s) = rx.recv().await {
            let _ = ble_tx.send(s).await;
        }
    });

    // Run the challenge (blocking this handler for up to 5 minutes).
    let (result_tx, result_rx) = tokio::sync::oneshot::channel();
    let challenger = state.challenger.clone();
    tokio::spawn(async move {
        let mut rx2 = rx;
        let result = challenger.run_challenge(req, &mut rx2).await;
        let _ = result_tx.send(result);
    });

    match result_rx.await {
        Ok(Ok(true))  => Json(serde_json::json!({ "status": "proven" })),
        Ok(Ok(false)) => Json(serde_json::json!({ "status": "timeout" })),
        _             => Json(serde_json::json!({ "status": "error" })),
    }
}

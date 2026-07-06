mod bot;
mod hone;
mod config;
mod contract;
mod db;
mod oracle;

use anyhow::Result;
use axum::{routing::get, Json, Router};
use hone::{HoneInferenceClient, HoneServiceClient};
use config::Config;
use contract::ContractService;
use db::DbClient;
use oracle::{EspnScraper, ScoreResolver};
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

/// Shared application state passed to every handler.
pub struct AppState {
    pub config: Config,
    pub contract: Arc<ContractService>,
    pub db: Result<DbClient, String>,
    pub hone: Option<HoneInferenceClient>,
    pub hone_service: Option<HoneServiceClient>,
    pub espn: Arc<EspnScraper>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("betchu_bot=info,warn")),
        )
        .init();

    let config = Config::from_env()?;
    info!("Starting betchu-bot v{}", env!("CARGO_PKG_VERSION"));
    info!("Contract: {}", config.bet_contract_address);
    info!("RPC: {}", config.rpc_url);
    info!("HONE: {}", config.hone_api_url);

    // ── Contract service ─────────────────────────────────────────────────────
    let contract = Arc::new(
        ContractService::new(
            config.rpc_url.clone(),
            config.bet_contract_address.clone(),
            &config.private_key,
        )
        .map_err(|e| {
            warn!("Contract init failed: {}", e);
            e
        })?,
    );

    // ── MongoDB ──────────────────────────────────────────────────────────────
    let db = match DbClient::connect(&config.mongodb_uri).await {
        Ok(d) => Ok(d),
        Err(e) => {
            warn!("MongoDB unavailable — running without persistence: {}", e);
            Err(e.to_string())
        }
    };

    // ── HONE inference client ───────────────────────────────────────────────
    let hone = if !config.hone_project_key.is_empty() {
        Some(HoneInferenceClient::new(
            config.hone_api_url.clone(),
            config.hone_project_key.clone(),
        ))
    } else {
        warn!("HONE_PROJECT_KEY not set — AI insights disabled");
        None
    };

    // ── HONE service client (heartbeat + registration) ──────────────────────
    let hone_service = if !config.hone_project_key.is_empty() {
        Some(HoneServiceClient::new(
            config.hone_api_url.clone(),
            config.hone_project_key.clone(),
        ))
    } else {
        None
    };

    // ── ESPN scraper ─────────────────────────────────────────────────────────
    let espn = Arc::new(EspnScraper::new());

    // ── App state ─────────────────────────────────────────────────────────────
    let state = Arc::new(AppState {
        config: config.clone(),
        contract: Arc::clone(&contract),
        db,
        hone,
        hone_service,
        espn: Arc::clone(&espn),
    });

    // ── HONE service registration ───────────────────────────────────────────
    let service_slug = if let Some(svc) = &state.hone_service {
        match svc.register_service(&config.hone_account).await {
            Ok(slug) => {
                info!("Registered as HONE service: {}", slug);
                slug
            }
            Err(e) => {
                warn!("Service registration failed: {}", e);
                format!("{}/betchu-bot", config.hone_account)
            }
        }
    } else {
        format!("{}/betchu-bot", config.hone_account)
    };

    // ── Health HTTP server (HONE compliance) ────────────────────────────────
    let health_state = Arc::clone(&state);
    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/", get(root_handler))
        .layer(CorsLayer::permissive())
        .with_state(health_state);

    let service_port = config.service_port;
    tokio::spawn(async move {
        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", service_port))
            .await
            .expect("Failed to bind health server");
        info!("Health server on port {}", service_port);
        axum::serve(listener, app).await.expect("Health server failed");
    });

    // ── HONE heartbeat loop (every 30s = one epoch) ─────────────────────────
    if state.hone_service.is_some() {
        let heartbeat_state = Arc::clone(&state);
        let slug = service_slug.clone();
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(30));
            loop {
                ticker.tick().await;
                if let Some(svc) = &heartbeat_state.hone_service {
                    let _ = svc.heartbeat(&slug, 0).await;
                }
            }
        });
        info!("HONE heartbeat started (30s interval) for {}", service_slug);
    }

    // ── ESPN score resolution cron (every 15 min) ────────────────────────────
    {
        let cron_state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut ticker = interval(Duration::from_secs(900)); // 15 minutes
            loop {
                ticker.tick().await;
                info!("Score cron: checking for resolved games...");
                let resolver = ScoreResolver::new(
                    Arc::clone(&cron_state.espn),
                    Arc::clone(&cron_state.contract),
                );
                match resolver.check_and_resolve().await {
                    Ok(ids) if !ids.is_empty() => {
                        info!("Oracle resolved pools: {:?}", ids);
                    }
                    Ok(_) => {}
                    Err(e) => warn!("Score resolution error: {}", e),
                }
            }
        });
    }

    // ── Telegram bot ─────────────────────────────────────────────────────────
    let bot = teloxide::Bot::new(&config.telegram_bot_token);
    info!("Telegram bot starting...");
    bot::run(bot, state).await;

    Ok(())
}

async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "betchu-bot",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn root_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": "betchu-bot",
        "description": "HONE-native P2P sports betting bot",
        "version": env!("CARGO_PKG_VERSION"),
        "hone_compliant": true,
        "endpoints": ["/health"],
    }))
}

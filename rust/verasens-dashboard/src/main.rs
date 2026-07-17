use std::sync::Arc;

use anyhow::Result;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use verasens_dashboard::agg_client::HttpAggregationClient;
use verasens_dashboard::app::AppState;
use verasens_dashboard::config::Config;
use verasens_dashboard::routes;
use verasens_dashboard::store::mem::MemStore;
use verasens_dashboard::store::pg::PgStore;
use verasens_dashboard::store::Store;
use verasens_dashboard::stripe_client::http::HttpStripeClient;
use verasens_dashboard::stripe_client::mock::MockStripeClient;
use verasens_dashboard::stripe_client::StripeClient;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("verasens_dashboard=info".parse()?)
                .add_directive("tower_http=warn".parse()?),
        )
        .init();

    let cfg = Config::from_env();

    let store: Arc<dyn Store> = match &cfg.database_url {
        Some(url) => {
            info!("connecting to Postgres");
            let pg = PgStore::connect(url).await?;
            pg.migrate().await?;
            Arc::new(pg)
        }
        None => {
            warn!("DATABASE_URL not set — using in-memory store (NOT for production; state is lost on restart)");
            Arc::new(MemStore::new())
        }
    };

    let stripe: Arc<dyn StripeClient> = match HttpStripeClient::from_env() {
        Some(client) => {
            info!("Stripe: live HTTP client configured from STRIPE_SECRET_KEY");
            Arc::new(client)
        }
        None => {
            warn!("STRIPE_SECRET_KEY not set — using mock Stripe client (NOT for production; no real charges)");
            MockStripeClient::new()
        }
    };

    let agg = Arc::new(HttpAggregationClient::new(cfg.aggregation_base_url.clone()));

    let port = cfg.port;
    let state = AppState::new(store, stripe, agg, cfg);

    let cors = CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any);

    let app = routes::router(state).layer(TraceLayer::new_for_http()).layer(cors);

    let addr = format!("0.0.0.0:{port}");
    info!("verasens-dashboard listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

mod app;
mod auth;
mod bonding_curve;
mod config;
mod ledger;
mod models;
mod notify;
mod routes;
mod state;

use anyhow::Result;
use axum::{
    middleware,
    routing::{delete, get, patch, post},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

use app::AppState;
use config::Config;
use state::new_shared_state;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("btcpc_market=info".parse()?)
                .add_directive("tower_http=warn".parse()?)
        )
        .init();

    let cfg = Config::from_env();
    let state = new_shared_state();

    let block_count  = ledger::load_block_files(&cfg, &state);
    let pending_count = ledger::load_pending_entries(&cfg, &state);
    info!("BTCPC Market v{} — replayed {} block + {} pending entries",
        env!("CARGO_PKG_VERSION"), block_count, pending_count);

    let port = cfg.port;
    let app_state = AppState::new(cfg, state);

    // ── Escrow auto-cancel sweep — runs every 60 s ─────────────────────────────
    tokio::spawn({
        let sweep_state = app_state.state.clone();
        let sweep_cfg   = app_state.cfg.clone();   // Arc<Config>
        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                let epoch = models::current_epoch();
                let expired: Vec<(String, String)> = {
                    let st = sweep_state.read();
                    st.orders.values()
                        .filter(|o| {
                            matches!(o.status.as_str(), "pending" | "placed")
                                && o.fulfill_deadline_epoch > 0
                                && epoch > o.fulfill_deadline_epoch
                        })
                        .map(|o| (o.order_id.clone(), o.buyer.clone()))
                        .collect()
                };
                for (oid, buyer) in expired {
                    if buyer.is_empty() { continue; }
                    let mut entry = models::LedgerEntry::new("ORDER_CANCEL", &buyer, epoch);
                    entry.order_data = Some(serde_json::json!({
                        "order_id": oid,
                        "reason": "fulfill_deadline_expired",
                    }));
                    match ledger::persist(&sweep_cfg, &sweep_state, &entry) {
                        Ok(_) => info!("Escrow sweep: auto-cancelled order {oid} at epoch {epoch}"),
                        Err(e) => warn!("Escrow sweep: failed to cancel order {oid}: {e}"),
                    }
                }
            }
        }
    });

    // ── Protected routes — require JWT or posting key ──────────────────────
    let auth_mw = middleware::from_fn_with_state(
        app_state.clone(),
        auth::require_auth,
    );

    let protected = Router::new()
        // Media upload (authenticated)
        .route("/media/upload",  post(routes::media::upload_image))
        // Stores (mutations)
        .route("/stores",                              post(routes::stores::open_store))
        .route("/stores/:seller",                      patch(routes::stores::update_store)
                                                       .delete(routes::stores::close_store))
        .route("/stores/:seller/shipping",             post(routes::stores::link_shipping))
        .route("/stores/:seller/shipping/:carrier",    delete(routes::stores::unlink_shipping))
        .route("/stores/:seller/tor/setup",            post(routes::stores::tor_setup))
        .route("/stores/:seller/tor",                  delete(routes::stores::tor_disable))
        // Store follow / unfollow
        .route("/stores/:seller/follow",               post(routes::follow::follow_store)
                                                       .delete(routes::follow::unfollow_store))
        // Products (mutations)
        .route("/products",           post(routes::products::create_product))
        .route("/products/*pid",      patch(routes::products::update_product)
                                      .delete(routes::products::delist_product)
                                      .post(routes::products::bump_product))
        // Orders (all authenticated — order data is private)
        .route("/orders",                    post(routes::orders::place_order))
        .route("/orders/my",                 get(routes::orders::my_orders))
        .route("/orders/:oid",               get(routes::orders::get_order))
        .route("/orders/:oid/fulfill",       post(routes::orders::fulfill_order))
        .route("/orders/:oid/deliver",       post(routes::orders::deliver_order))
        .route("/orders/:oid/cancel",        post(routes::orders::cancel_order))
        .route("/orders/:oid/dispute",       post(routes::orders::dispute_order))
        .route("/escrow/balance",            get(routes::orders::escrow_balance))
        // Offers
        .route("/offers",                    post(routes::offers::place_offer))
        .route("/offers/my",                 get(routes::offers::my_offers))
        .route("/offers/:oid/counter",       post(routes::offers::counter_offer))
        .route("/offers/:oid/accept",        post(routes::offers::accept_offer))
        .route("/offers/:oid/reject",        post(routes::offers::reject_offer))
        // Wishlist
        .route("/wishlist",                  get(routes::wishlist::get_wishlist))
        .route("/wishlist/:pid",             post(routes::wishlist::add_wishlist)
                                             .delete(routes::wishlist::remove_wishlist))
        // Feed
        .route("/feed",                      get(routes::follow::get_feed))
        // Reputation
        .route("/reputation/vote",           post(routes::reputation::vote))
        .route("/reputation/:vid/reply",     post(routes::reputation::reply_to_vote))
        // Q&A (mutations)
        .route("/products/:seller/:slug/qa",            post(routes::qa::ask_question))
        .route("/products/:seller/:slug/qa/:qa_id",     patch(routes::qa::answer_question))
        // Accounts (memo key registration)
        .route("/accounts/memo-key",                    post(routes::accounts::register_memo_key))
        .route_layer(auth_mw);

    // ── Public routes — no auth required ──────────────────────────────────
    let public = Router::new()
        .route("/auth/login",                  post(routes::auth::login))
        .route("/media/:hash",                 get(routes::media::serve_image))
        .route("/stores",                      get(routes::stores::list_stores))
        .route("/stores/:seller",              get(routes::stores::get_store))
        .route("/stores/:seller/metrics",      get(routes::stores::get_store_metrics))
        .route("/products",                    get(routes::products::list_products))
        .route("/products/*pid",               get(routes::products::get_product))
        .route("/categories",                  get(routes::products::list_categories))
        .route("/quote/capacity",              get(routes::stores::quote_capacity))
        .route("/import/amazon",               post(routes::import::import_amazon))
        .route("/wishlist/count/:pid",         get(routes::wishlist::wishlist_count))
        // Q&A (public read)
        .route("/products/:seller/:slug/qa",   get(routes::qa::list_questions))
        // Accounts (memo key lookup)
        .route("/accounts/:username/memo-key", get(routes::accounts::get_memo_key));

    let commerce = protected.merge(public);

    let app = Router::new()
        .nest("/api/commerce", commerce)
        .route("/health", get(health))
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let addr = format!("0.0.0.0:{port}");
    info!("BTCPC Market listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health() -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "ok": true,
        "service": "btcpc-market",
        "version": env!("CARGO_PKG_VERSION"),
        "epoch": models::current_epoch(),
    }))
}

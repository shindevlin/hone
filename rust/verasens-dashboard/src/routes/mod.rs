pub mod auth_routes;
pub mod keys;
pub mod query;
pub mod usage;

use axum::routing::{get, post};
use axum::Router;

use crate::app::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/dashboard/health", get(health))
        .route("/api/dashboard/signup", post(auth_routes::signup))
        .route("/api/dashboard/login", post(auth_routes::login))
        .route("/api/dashboard/totp/enroll", post(auth_routes::totp_enroll))
        .route("/api/dashboard/totp/verify", post(auth_routes::totp_verify))
        .route("/api/dashboard/logout", post(auth_routes::logout))
        .route("/api/dashboard/keys", post(keys::create_key).get(keys::list_keys))
        .route("/api/dashboard/keys/:id/revoke", post(keys::revoke_key))
        .route("/api/dashboard/query", get(query::run_query))
        .route("/api/dashboard/usage", get(usage::usage_history))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

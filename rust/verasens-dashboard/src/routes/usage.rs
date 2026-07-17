//! Usage history view — session-authenticated (dashboard UI) or API-key
//! authenticated with `usage:read` scope.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};
use serde::Deserialize;

use crate::app::AppState;
use crate::models::Scope;
use crate::routes::auth_routes::require_session;
use crate::routes::keys::require_api_key;

#[derive(Deserialize, Default)]
pub struct UsageQuery {
    pub limit: Option<i64>,
}

pub async fn usage_history(State(state): State<AppState>, headers: HeaderMap, Query(q): Query<UsageQuery>) -> impl IntoResponse {
    let org_id = if headers.get(axum::http::header::AUTHORIZATION).is_some() {
        match require_api_key(&state, &headers, Scope::UsageRead).await {
            Ok(key) => key.org_id,
            Err(status) => return (status, Json(serde_json::json!({"error": "unauthorized"}))).into_response(),
        }
    } else {
        match require_session(&state, &headers).await {
            Ok(session) => session.org_id,
            Err(status) => return (status, Json(serde_json::json!({"error": "unauthorized"}))).into_response(),
        }
    };

    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    match state.store.list_usage_history(org_id, limit).await {
        Ok(events) => (StatusCode::OK, Json(serde_json::json!({"events": events}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

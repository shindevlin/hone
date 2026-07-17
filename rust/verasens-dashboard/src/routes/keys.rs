//! API key issuance / listing / revocation. Session-authenticated only
//! (owner/admin) — the whole point of the key path is that keys, not
//! cookies, are what get handed to machines.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};
use axum::Json as JsonExtract;
use serde::Deserialize;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::generate_api_key;
use crate::models::{Role, Scope};
use crate::routes::auth_routes::require_session;

#[derive(Deserialize)]
pub struct CreateKeyRequest {
    pub scopes: Vec<String>,
}

pub async fn create_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    JsonExtract(req): JsonExtract<CreateKeyRequest>,
) -> impl IntoResponse {
    let session = match require_session(&state, &headers).await {
        Ok(s) => s,
        Err(status) => return (status, Json(serde_json::json!({"error": "unauthorized"}))).into_response(),
    };

    let member = match state.store.get_member(session.member_id).await {
        Ok(m) => m,
        Err(_) => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "unauthorized"}))).into_response(),
    };
    let role = Role::parse(&member.role).unwrap_or(Role::Analyst);
    if !role.can_manage_seats_and_billing() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "owner/admin only"}))).into_response();
    }

    let mut valid_scopes = Vec::new();
    for s in &req.scopes {
        match Scope::parse(s) {
            Some(scope) => valid_scopes.push(scope.as_str().to_string()),
            None => {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": format!("invalid scope: {s}")})))
                    .into_response()
            }
        }
    }
    if valid_scopes.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "at least one scope required"}))).into_response();
    }

    let new_key = generate_api_key();
    let stored = match state
        .store
        .create_api_key(session.org_id, &new_key.prefix, &new_key.hash, &valid_scopes, member.id)
        .await
    {
        Ok(k) => k,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    };

    // The raw secret is returned exactly once, here, and never again —
    // neither stored nor logged.
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": stored.id,
            "key": new_key.raw,
            "prefix": stored.prefix,
            "scopes": stored.scopes,
        })),
    )
        .into_response()
}

pub async fn list_keys(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let session = match require_session(&state, &headers).await {
        Ok(s) => s,
        Err(status) => return (status, Json(serde_json::json!({"error": "unauthorized"}))).into_response(),
    };

    match state.store.list_api_keys(session.org_id).await {
        Ok(keys) => {
            let out: Vec<_> = keys
                .into_iter()
                .map(|k| {
                    serde_json::json!({
                        "id": k.id,
                        "prefix": k.prefix,
                        "scopes": k.scopes,
                        "created_at": k.created_at,
                        "last_used_at": k.last_used_at,
                        "revoked_at": k.revoked_at,
                        "active": k.is_active(),
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"keys": out}))).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response(),
    }
}

pub async fn revoke_key(State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> impl IntoResponse {
    let session = match require_session(&state, &headers).await {
        Ok(s) => s,
        Err(status) => return (status, Json(serde_json::json!({"error": "unauthorized"}))).into_response(),
    };

    let member = match state.store.get_member(session.member_id).await {
        Ok(m) => m,
        Err(_) => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "unauthorized"}))).into_response(),
    };
    let role = Role::parse(&member.role).unwrap_or(Role::Analyst);
    if !role.can_manage_seats_and_billing() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "owner/admin only"}))).into_response();
    }

    match state.store.revoke_api_key(id, session.org_id).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({"ok": true}))).into_response(),
        Err(_) => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "key not found"}))).into_response(),
    }
}

/// Resolve an `Authorization: Bearer btcpc_live_...` header to an active
/// `ApiKey` with the required scope. This is the machine-credential path —
/// structurally separate from `require_session` and never accepts a
/// session cookie.
pub async fn require_api_key(
    state: &AppState,
    headers: &HeaderMap,
    required_scope: Scope,
) -> Result<crate::models::ApiKey, StatusCode> {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let raw = auth_header.strip_prefix("Bearer ").ok_or(StatusCode::UNAUTHORIZED)?;
    let prefix = crate::auth::prefix_of(raw).ok_or(StatusCode::UNAUTHORIZED)?;

    let key = state.store.get_api_key_by_prefix(&prefix).await.map_err(|_| StatusCode::UNAUTHORIZED)?;

    if !key.is_active() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if !crate::auth::verify_api_key(raw, &key.hash) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if !key.has_scope(required_scope) {
        return Err(StatusCode::FORBIDDEN);
    }

    let _ = state.store.touch_api_key_last_used(key.id, chrono::Utc::now()).await;
    Ok(key)
}

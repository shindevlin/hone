use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::{
    app::AppState,
    auth::AuthUser,
    ledger,
    models::{current_epoch, LedgerEntry, RegisterMemoKeyRequest},
};

/// POST /api/commerce/accounts/memo-key — register a memo pubkey (auth required)
pub async fn register_memo_key(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<RegisterMemoKeyRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if body.memo_pubkey.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "memo_pubkey required"}))));
    }
    let actor = user.username;
    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("MEMO_KEY_REGISTER", &actor, epoch);
    entry.memo_key_data = Some(json!({ "memo_pubkey": body.memo_pubkey }));
    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    Ok(Json(json!({ "ok": true, "username": actor, "memo_pubkey": body.memo_pubkey })))
}

/// GET /api/commerce/accounts/:username/memo-key — fetch a user's memo pubkey (public)
pub async fn get_memo_key(
    State(app): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let state = app.state.read();
    match state.memo_keys.get(&username) {
        Some(pk) => Ok(Json(json!({ "username": username, "memo_pubkey": pk }))),
        None => Err((StatusCode::NOT_FOUND, Json(json!({"error": "no memo key registered"})))),
    }
}

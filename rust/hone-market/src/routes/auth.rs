use axum::{extract::State, http::StatusCode, Json};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{app::AppState, auth::Claims};

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username:    String,
    pub posting_key: String,
}

/// POST /api/commerce/auth/login
/// Exchange a posting key (account:hex64) for a short-lived JWT.
/// The posting key is validated for format only — this service is a local
/// sidecar and trusts the operator's node to manage actual key issuance.
pub async fn login(
    State(app): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let username = body.username.trim().to_lowercase();
    let key_hex  = body.posting_key.trim().to_lowercase();

    if username.is_empty() {
        return Err((StatusCode::BAD_REQUEST,
            Json(json!({"error": "username required"}))));
    }
    if !username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err((StatusCode::BAD_REQUEST,
            Json(json!({"error": "username may only contain letters, numbers, _ and -"}))));
    }
    if key_hex.len() != 64 {
        return Err((StatusCode::UNAUTHORIZED,
            Json(json!({"error": "posting key must be exactly 64 hex characters"}))));
    }
    if hex::decode(&key_hex).is_err() {
        return Err((StatusCode::UNAUTHORIZED,
            Json(json!({"error": "posting key must be valid hex"}))));
    }

    let exp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + 86_400; // 24 h

    let claims = Claims { username: username.clone(), exp };
    let token  = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(app.jwt_secret.as_bytes()),
    )
    .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "token signing failed"}))))?;

    Ok(Json(json!({
        "ok":         true,
        "token":      token,
        "username":   username,
        "expires_in": 86400,
        "key_type":   "posting",
    })))
}

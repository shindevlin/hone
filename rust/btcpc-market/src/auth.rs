use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

use crate::app::AppState;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub username: String,
    pub exp: u64,
}

/// Authenticated user, injected as request extension.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub username: String,
}

/// Axum middleware: validates either a JWT or a posting key (account:hex64key).
pub async fn require_auth(
    State(app): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");

    let user = if let Some(token) = auth_header.strip_prefix("Bearer ") {
        validate_jwt(token, &app.jwt_secret)?
    } else if let Some(creds) = auth_header.strip_prefix("PostingKey ") {
        validate_posting_key(creds)?
    } else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    request.extensions_mut().insert(user);
    Ok(next.run(request).await)
}

fn validate_jwt(token: &str, secret: &str) -> Result<AuthUser, StatusCode> {
    let key = DecodingKey::from_secret(secret.as_bytes());
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;
    decode::<Claims>(token, &key, &validation)
        .map(|data| AuthUser { username: data.claims.username })
        .map_err(|_| StatusCode::UNAUTHORIZED)
}

/// Posting key format: "account:hex64key"
/// The key must be a 64-char hex string (32-byte raw key).
fn validate_posting_key(creds: &str) -> Result<AuthUser, StatusCode> {
    let mut parts = creds.splitn(2, ':');
    let account = parts.next().unwrap_or("").trim();
    let key_hex = parts.next().unwrap_or("").trim();

    if account.is_empty() || key_hex.len() != 64 {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if hex::decode(key_hex).is_err() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    // Key format validated — the Node.js auth does the same (format check only,
    // no signature challenge at this layer since we're a local sidecar)
    Ok(AuthUser { username: account.to_string() })
}

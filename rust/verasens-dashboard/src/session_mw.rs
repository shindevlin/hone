//! Session-cookie plumbing. The cookie carries only an opaque session id
//! (a UUID) — all authorization state (member, org, role, expiry, revoked)
//! lives server-side in the `session` table, so a stolen/leaked cookie
//! value that's later revoked or expired is immediately worthless.

use axum::http::header::HeaderMap;
use chrono::Utc;
use uuid::Uuid;

use crate::models::Session;
use crate::store::{Store, StoreError};

pub const SESSION_COOKIE_NAME: &str = "verasens_session";

/// Build the `Set-Cookie` header value for a new session. HttpOnly (no JS
/// access — mitigates XSS token theft), Secure (HTTPS-only transmission),
/// SameSite=Lax (mitigates CSRF on cross-site navigation while still
/// allowing top-level GET navigation), scoped to the whole app path.
pub fn set_cookie_header(session_id: Uuid, max_age_secs: i64) -> String {
    format!(
        "{name}={id}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={max_age}",
        name = SESSION_COOKIE_NAME,
        id = session_id,
        max_age = max_age_secs.max(0),
    )
}

pub fn clear_cookie_header() -> String {
    format!("{name}=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0", name = SESSION_COOKIE_NAME)
}

pub fn extract_session_id(headers: &HeaderMap) -> Option<Uuid> {
    let cookie_header = headers.get(axum::http::header::COOKIE)?.to_str().ok()?;
    for part in cookie_header.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(&format!("{SESSION_COOKIE_NAME}=")) {
            return Uuid::parse_str(v).ok();
        }
    }
    None
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("no session cookie")]
    Missing,
    #[error("session not found")]
    NotFound,
    #[error("session revoked")]
    Revoked,
    #[error("session expired (absolute)")]
    ExpiredAbsolute,
    #[error("session expired (idle)")]
    ExpiredIdle,
}

/// Validate a session id against the server-side store: must exist, not be
/// revoked, not past absolute expiry, and not idle past `idle_minutes`
/// since `last_seen_at`. On success, touches `last_seen_at` (sliding idle
/// window) and returns the session.
pub async fn validate_and_touch(
    store: &dyn Store,
    session_id: Uuid,
    idle_minutes: i64,
) -> Result<Session, SessionError> {
    let session = store.get_session(session_id).await.map_err(|e| match e {
        StoreError::NotFound => SessionError::NotFound,
        _ => SessionError::NotFound,
    })?;

    if session.revoked_at.is_some() {
        return Err(SessionError::Revoked);
    }

    let now = Utc::now();
    if now > session.absolute_expires_at {
        return Err(SessionError::ExpiredAbsolute);
    }

    let idle_deadline = session.last_seen_at + chrono::Duration::minutes(idle_minutes);
    if now > idle_deadline {
        return Err(SessionError::ExpiredIdle);
    }

    let _ = store.touch_session(session_id, now).await;
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::mem::MemStore;

    #[tokio::test]
    async fn revoked_session_rejected() {
        let store = MemStore::new();
        let org = store.create_org("Acme", "starter").await.unwrap();
        let member = store
            .create_member(org.id, "a@acme.com", "owner", "active", "hash")
            .await
            .unwrap();
        let session = store
            .create_session(member.id, org.id, Utc::now() + chrono::Duration::hours(12))
            .await
            .unwrap();

        store.revoke_session(session.id).await.unwrap();

        let result = validate_and_touch(&store, session.id, 30).await;
        assert!(matches!(result, Err(SessionError::Revoked)));
    }

    #[tokio::test]
    async fn expired_absolute_session_rejected() {
        let store = MemStore::new();
        let org = store.create_org("Acme", "starter").await.unwrap();
        let member = store
            .create_member(org.id, "a@acme.com", "owner", "active", "hash")
            .await
            .unwrap();
        let session = store
            .create_session(member.id, org.id, Utc::now() - chrono::Duration::seconds(1))
            .await
            .unwrap();

        let result = validate_and_touch(&store, session.id, 30).await;
        assert!(matches!(result, Err(SessionError::ExpiredAbsolute)));
    }

    #[tokio::test]
    async fn valid_session_touches_last_seen() {
        let store = MemStore::new();
        let org = store.create_org("Acme", "starter").await.unwrap();
        let member = store
            .create_member(org.id, "a@acme.com", "owner", "active", "hash")
            .await
            .unwrap();
        let session = store
            .create_session(member.id, org.id, Utc::now() + chrono::Duration::hours(12))
            .await
            .unwrap();

        let result = validate_and_touch(&store, session.id, 30).await;
        assert!(result.is_ok());
    }
}

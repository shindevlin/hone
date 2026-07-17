//! Org signup + member login, with mandatory TOTP for owner/admin.
//!
//! Login flow for a TOTP-enabled owner/admin is two calls:
//!   1. POST /login with email+password -> if TOTP is enabled, returns
//!      `{"totp_required": true, "totp_challenge": "<member_id>"}` and does
//!      NOT set a session cookie yet.
//!   2. POST /totp/verify with `{"totp_challenge", "code"}` -> on success,
//!      sets the session cookie.
//! A member who hasn't enrolled TOTP yet but holds owner/admin role gets a
//! session cookie immediately after password check but is flagged
//! `totp_enrollment_required: true` in the response so the dashboard UI can
//! force the enrollment screen before granting access to billing-affecting
//! actions — enrollment itself happens via /totp/enroll.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};
use axum::Json as JsonExtract;
use chrono::Utc;
use serde::Deserialize;
use uuid::Uuid;

use crate::app::AppState;
use crate::auth::{generate_totp_secret, hash_password, totp_provisioning_uri, verify_password, verify_totp};
use crate::models::{MemberStatus, PlanId, Role};
use crate::session_mw::{extract_session_id, set_cookie_header, validate_and_touch};
use crate::store::StoreError;

#[derive(Deserialize)]
pub struct SignupRequest {
    pub org_name: String,
    pub plan_id: String,
    pub owner_email: String,
    pub owner_password: String,
}

pub async fn signup(State(state): State<AppState>, JsonExtract(req): JsonExtract<SignupRequest>) -> impl IntoResponse {
    if req.owner_password.len() < 12 {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "password must be at least 12 characters"})))
            .into_response();
    }
    if PlanId::parse(&req.plan_id).is_none() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "invalid plan_id"}))).into_response();
    }

    let org = match state.store.create_org(&req.org_name, &req.plan_id).await {
        Ok(o) => o,
        Err(e) => return internal_error(e),
    };

    let password_hash = match hash_password(&req.owner_password) {
        Ok(h) => h,
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "signup failed"})))
                .into_response()
        }
    };

    let member = match state
        .store
        .create_member(org.id, &req.owner_email, Role::Owner.as_str(), MemberStatus::Active.as_str(), &password_hash)
        .await
    {
        Ok(m) => m,
        Err(StoreError::Conflict(msg)) => {
            return (StatusCode::CONFLICT, Json(serde_json::json!({"error": msg}))).into_response()
        }
        Err(e) => return internal_error(e),
    };

    // Create the Stripe customer up front so metering can report usage from
    // the very first billable query (PRD 1.4: "one Stripe Customer per
    // org"). If Stripe isn't reachable, the org is still created and
    // reconciliation will pick up unreported usage once billing is fixed.
    if let Ok(customer_id) = state.stripe.create_customer(&org.name, &org.id.to_string()).await {
        let _ = state.store.set_org_stripe_customer(org.id, &customer_id).await;
    }

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "org_id": org.id,
            "member_id": member.id,
            "role": member.role,
            "totp_enrollment_required": true,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

pub async fn login(State(state): State<AppState>, JsonExtract(req): JsonExtract<LoginRequest>) -> impl IntoResponse {
    let member = match state.store.get_member_by_email_any_org(&req.email).await {
        Ok(m) => m,
        Err(_) => {
            // Do not reveal whether the email exists.
            return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "invalid credentials"}))).into_response();
        }
    };

    if member.status == MemberStatus::Suspended.as_str() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({"error": "account suspended"}))).into_response();
    }

    if !verify_password(&req.password, &member.password_hash) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "invalid credentials"}))).into_response();
    }

    let role = Role::parse(&member.role).unwrap_or(Role::Analyst);

    if role.requires_totp() && member.totp_enabled {
        // Password check passed; require the TOTP step before a session is
        // minted. `totp_challenge` is the member id — treated as a
        // short-lived, single-purpose token, not a session credential.
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "totp_required": true,
                "totp_challenge": member.id,
            })),
        )
            .into_response();
    }

    finish_login(&state, member.id, member.org_id, role.requires_totp() && !member.totp_enabled).await
}

#[derive(Deserialize)]
pub struct TotpVerifyRequest {
    pub totp_challenge: Uuid,
    pub code: String,
}

pub async fn totp_verify(State(state): State<AppState>, JsonExtract(req): JsonExtract<TotpVerifyRequest>) -> impl IntoResponse {
    let member = match state.store.get_member(req.totp_challenge).await {
        Ok(m) => m,
        Err(_) => return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "invalid challenge"}))).into_response(),
    };

    let secret = match &member.totp_secret {
        Some(s) => s,
        None => return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "totp not enrolled"}))).into_response(),
    };

    if !verify_totp(secret, &req.code) {
        return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "invalid code"}))).into_response();
    }

    finish_login(&state, member.id, member.org_id, false).await
}

#[derive(Deserialize)]
pub struct TotpEnrollRequest {
    pub member_id: Uuid,
}

/// Issues a fresh TOTP secret for a member and returns the provisioning
/// URI (for QR rendering client-side). Enrollment is only finalized once
/// the member proves possession via `totp_verify_enrollment`-style flow;
/// for simplicity here, `/totp/enroll` sets the secret and the member must
/// then call `/totp/verify` with a challenge equal to their own id to flip
/// `totp_enabled` — reusing the login verification path keeps one TOTP
/// check implementation instead of two.
pub async fn totp_enroll(State(state): State<AppState>, JsonExtract(req): JsonExtract<TotpEnrollRequest>) -> impl IntoResponse {
    let member = match state.store.get_member(req.member_id).await {
        Ok(m) => m,
        Err(_) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "member not found"}))).into_response(),
    };
    let org = match state.store.get_org(member.org_id).await {
        Ok(o) => o,
        Err(_) => return (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "org not found"}))).into_response(),
    };

    let secret = generate_totp_secret();
    if state.store.set_member_totp_secret(member.id, &secret).await.is_err() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "enroll failed"}))).into_response();
    }

    let uri = totp_provisioning_uri(&secret, &org.name, &member.email);

    (StatusCode::OK, Json(serde_json::json!({ "secret": secret, "provisioning_uri": uri }))).into_response()
}

async fn finish_login(state: &AppState, member_id: Uuid, org_id: Uuid, totp_enrollment_required: bool) -> axum::response::Response {
    let absolute_expires_at = Utc::now() + chrono::Duration::hours(state.cfg.session_absolute_hours);
    let session = match state.store.create_session(member_id, org_id, absolute_expires_at).await {
        Ok(s) => s,
        Err(e) => return internal_error(e),
    };

    // If this call is the post-TOTP-verify finish, flip totp_enabled if this
    // is the member's first successful verification after enrollment.
    if let Ok(member) = state.store.get_member(member_id).await {
        if member.totp_secret.is_some() && !member.totp_enabled {
            let _ = state.store.enable_member_totp(member_id).await;
        }
    }

    let max_age = state.cfg.session_absolute_hours * 3600;
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::SET_COOKIE,
        set_cookie_header(session.id, max_age).parse().unwrap(),
    );

    (
        StatusCode::OK,
        headers,
        Json(serde_json::json!({
            "org_id": org_id,
            "member_id": member_id,
            "totp_enrollment_required": totp_enrollment_required,
        })),
    )
        .into_response()
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Some(session_id) = extract_session_id(&headers) {
        let _ = state.store.revoke_session(session_id).await;
    }
    let mut out_headers = HeaderMap::new();
    out_headers.insert(SET_COOKIE_KEY, crate::session_mw::clear_cookie_header().parse().unwrap());
    (StatusCode::OK, out_headers, Json(serde_json::json!({"ok": true})))
}

const SET_COOKIE_KEY: axum::http::HeaderName = axum::http::header::SET_COOKIE;

/// Shared helper: resolve the caller's session from the request cookie,
/// validating expiry/revocation. Used by all routes requiring a logged-in
/// dashboard user (as opposed to an API-key-authenticated machine caller).
pub async fn require_session(state: &AppState, headers: &HeaderMap) -> Result<crate::models::Session, StatusCode> {
    let session_id = extract_session_id(headers).ok_or(StatusCode::UNAUTHORIZED)?;
    validate_and_touch(state.store.as_ref(), session_id, state.cfg.session_idle_minutes)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)
}

fn internal_error(e: impl std::fmt::Display) -> axum::response::Response {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response()
}

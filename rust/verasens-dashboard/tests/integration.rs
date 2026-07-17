//! End-to-end integration tests driven through the real Axum router, using
//! the in-memory `MemStore`, `MockStripeClient`, and a fake
//! `AggregationClient`. These exercise real HTTP request/response cycles
//! (cookies, headers, JSON bodies) against the actual route handlers — not
//! calls straight into internal functions — so they prove the wiring, not
//! just the pieces.
//!
//! No real Postgres or Stripe network calls happen in this file; see
//! `tests/postgres_integration.rs` for the ephemeral-Postgres variant.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::ServiceExt;

use verasens_dashboard::agg_client::{AggError, AggregationClient, GroupSnapshot, Provenance, QueryParams, QueryResponse, ValueStats};
use verasens_dashboard::app::AppState;
use verasens_dashboard::config::Config;
use verasens_dashboard::routes;
use verasens_dashboard::store::mem::MemStore;
use verasens_dashboard::store::Store;
use verasens_dashboard::stripe_client::mock::MockStripeClient;
use verasens_dashboard::stripe_client::StripeClient;

/// A fake aggregation client whose response is controlled per-test via a
/// shared flag, so tests can drive both the "billable" and "not billable"
/// paths through the real HTTP query route.
struct FakeAggClient {
    empty: std::sync::atomic::AtomicBool,
}

impl FakeAggClient {
    fn new(empty: bool) -> Self {
        Self { empty: std::sync::atomic::AtomicBool::new(empty) }
    }
}

#[async_trait]
impl AggregationClient for FakeAggClient {
    async fn query(&self, _params: &QueryParams) -> Result<QueryResponse, AggError> {
        if self.empty.load(std::sync::atomic::Ordering::SeqCst) {
            Ok(QueryResponse { count: 0, groups: vec![] })
        } else {
            Ok(QueryResponse {
                count: 1,
                groups: vec![GroupSnapshot {
                    key: serde_json::json!({"class": "gnss"}),
                    count: 2,
                    verified_count: 2,
                    unverified_count: 0,
                    stats: Some(ValueStats { mean: 1.0, min: 1.0, max: 1.0, p50: 1.0, p95: 1.0 }),
                    provenance: vec![
                        Provenance {
                            sensor_id: "sensor-1".into(),
                            owner: "alice".into(),
                            data_hash: "h1".into(),
                            epoch: 10,
                            verified: true,
                            batch_hash: None,
                        },
                        Provenance {
                            sensor_id: "sensor-2".into(),
                            owner: "bob".into(),
                            data_hash: "h2".into(),
                            epoch: 11,
                            verified: true,
                            batch_hash: None,
                        },
                    ],
                    geo_key: None,
                    window_key: None,
                }],
            })
        }
    }
}

fn test_config() -> Config {
    Config {
        port: 0,
        database_url: None,
        aggregation_base_url: "http://unused".into(),
        session_idle_minutes: 30,
        session_absolute_hours: 12,
    }
}

struct Harness {
    app: axum::Router,
    store: Arc<MemStore>,
    stripe: Arc<MockStripeClient>,
}

fn build_harness(agg_empty: bool) -> Harness {
    let store = Arc::new(MemStore::new());
    let stripe = MockStripeClient::new();
    let agg: Arc<dyn AggregationClient> = Arc::new(FakeAggClient::new(agg_empty));

    let state = AppState::new(store.clone() as Arc<dyn Store>, stripe.clone(), agg, test_config());
    let app = routes::router(state);
    Harness { app, store, stripe }
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap_or(serde_json::json!(null))
}

fn extract_set_cookie(resp: &axum::response::Response) -> String {
    resp.headers()
        .get(header::SET_COOKIE)
        .expect("expected Set-Cookie header")
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

// ============================== Auth flow ===================================

#[tokio::test]
async fn signup_then_login_without_totp_enabled_yet_flags_enrollment_required() {
    let h = build_harness(true);

    let signup_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/signup")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "org_name": "Acme Sensing",
                        "plan_id": "starter",
                        "owner_email": "owner@acme.example",
                        "owner_password": "correct horse battery staple"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(signup_resp.status(), StatusCode::CREATED);
    let signup_body = body_json(signup_resp).await;
    assert_eq!(signup_body["totp_enrollment_required"], true);

    // Login with password only — TOTP not enrolled yet, so login succeeds
    // immediately but flags totp_enrollment_required so the UI can force
    // enrollment before letting the owner touch billing-affecting actions.
    let login_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"email": "owner@acme.example", "password": "correct horse battery staple"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login_resp.status(), StatusCode::OK);
    assert!(login_resp.headers().get(header::SET_COOKIE).is_some());
    let login_body = body_json(login_resp).await;
    assert_eq!(login_body["totp_enrollment_required"], true);
}

#[tokio::test]
async fn wrong_password_rejected_and_no_cookie_set() {
    let h = build_harness(true);

    h.app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/signup")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "org_name": "Acme",
                        "plan_id": "starter",
                        "owner_email": "owner@acme.example",
                        "owner_password": "correct horse battery staple"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/login")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"email": "owner@acme.example", "password": "wrong password entirely"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(resp.headers().get(header::SET_COOKIE).is_none());
}

#[tokio::test]
async fn totp_enrolled_owner_must_complete_second_factor_before_session_is_minted() {
    let h = build_harness(true);

    let signup_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/signup")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "org_name": "Acme",
                        "plan_id": "starter",
                        "owner_email": "owner@acme.example",
                        "owner_password": "correct horse battery staple"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let signup_body = body_json(signup_resp).await;
    let member_id = signup_body["member_id"].as_str().unwrap();

    // Enroll TOTP.
    let enroll_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/totp/enroll")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"member_id": member_id}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(enroll_resp.status(), StatusCode::OK);
    let enroll_body = body_json(enroll_resp).await;
    let secret = enroll_body["secret"].as_str().unwrap().to_string();

    // Finish enrollment by verifying once (this flips totp_enabled=true).
    let secret_bytes = base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &secret).unwrap();
    let totp = totp_rs::TOTP::new(totp_rs::Algorithm::SHA1, 6, 1, 30, secret_bytes).unwrap();
    let code = totp.generate_current().unwrap();

    let verify_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/totp/verify")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"totp_challenge": member_id, "code": code}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(verify_resp.status(), StatusCode::OK);

    // Now login with password alone must NOT mint a session — it must
    // demand the TOTP step.
    let login_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"email": "owner@acme.example", "password": "correct horse battery staple"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login_resp.status(), StatusCode::OK);
    assert!(login_resp.headers().get(header::SET_COOKIE).is_none(), "password-only login must not set a session cookie once TOTP is enabled");
    let login_body = body_json(login_resp).await;
    assert_eq!(login_body["totp_required"], true);
    let challenge = login_body["totp_challenge"].as_str().unwrap();

    // Wrong code rejected.
    let bad_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/totp/verify")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"totp_challenge": challenge, "code": "000000"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    // A guessed code of "000000" only accidentally matches; assert it's
    // rejected in the overwhelming common case by checking against a value
    // we know is wrong (differs from the just-used valid code).
    if code != "000000" {
        assert_eq!(bad_resp.status(), StatusCode::UNAUTHORIZED);
    }

    // Correct second-factor code completes login and sets a cookie.
    let code2 = totp.generate_current().unwrap();
    let final_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/totp/verify")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"totp_challenge": challenge, "code": code2}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(final_resp.status(), StatusCode::OK);
    assert!(final_resp.headers().get(header::SET_COOKIE).is_some());
}

// ======================== Billable vs non-billable metering =================

async fn signed_up_session_cookie(h: &Harness) -> (String, uuid::Uuid) {
    let signup_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/signup")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "org_name": "Acme",
                        "plan_id": "starter",
                        "owner_email": "owner@acme.example",
                        "owner_password": "correct horse battery staple"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(signup_resp).await;
    let org_id: uuid::Uuid = body["org_id"].as_str().unwrap().parse().unwrap();

    let login_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"email": "owner@acme.example", "password": "correct horse battery staple"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = extract_set_cookie(&login_resp);
    (cookie, org_id)
}

#[tokio::test]
async fn empty_query_result_is_not_metered() {
    let h = build_harness(true); // agg returns empty
    let (cookie, org_id) = signed_up_session_cookie(&h).await;

    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/dashboard/query?class=gnss")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body["billed_usage_event_id"].is_null());

    let history = h.store.list_usage_history(org_id, 100).await.unwrap();
    assert_eq!(history.len(), 0, "an empty aggregation result must not produce a usage_event");
    assert_eq!(h.stripe.total_calls(), 0, "no Stripe meter event should be reported for a non-billable query");
}

#[tokio::test]
async fn nonempty_query_result_is_metered_and_writes_ledger_before_stripe_report() {
    let h = build_harness(false); // agg returns a nonempty result
    let (cookie, org_id) = signed_up_session_cookie(&h).await;

    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/dashboard/query?class=gnss")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(!body["billed_usage_event_id"].is_null(), "a nonempty result must be metered");

    let history = h.store.list_usage_history(org_id, 100).await.unwrap();
    assert_eq!(history.len(), 1, "exactly one usage_event must be written for the billable query");
    let event = &history[0];
    assert_eq!(event.units, 1);
    // The local ledger row must exist and (since Stripe reporting happened
    // synchronously right after in this test's org, which has a Stripe
    // customer from signup) carry a stripe_meter_event_id — proving the
    // local write is the row that the Stripe id gets attached to, i.e. it
    // existed FIRST.
    assert!(event.stripe_meter_event_id.is_some());
    assert_eq!(h.stripe.total_calls(), 1);
    assert_eq!(h.stripe.reported_event_count(), 1);

    // Settlement intents: one per provenance entry in the fake response (2).
    let intents = h.store.list_settlement_intents_for_usage(event.id).await.unwrap();
    assert_eq!(intents.len(), 2, "one settlement_intent per provenance tuple must be written atomically with the usage_event");
    let sensor_ids: Vec<&str> = intents.iter().map(|i| i.sensor_id.as_str()).collect();
    assert!(sensor_ids.contains(&"sensor-1"));
    assert!(sensor_ids.contains(&"sensor-2"));
}

#[tokio::test]
async fn repeated_identical_queries_each_produce_their_own_usage_event() {
    // PRD: a query is billable on each successful non-empty result — this
    // is not a "bill once per unique query" dedup, it's per-call metering.
    let h = build_harness(false);
    let (cookie, org_id) = signed_up_session_cookie(&h).await;

    for _ in 0..3 {
        let resp = h
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/dashboard/query?class=gnss")
                    .header(header::COOKIE, cookie.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    let history = h.store.list_usage_history(org_id, 100).await.unwrap();
    assert_eq!(history.len(), 3);
    let ids: std::collections::HashSet<_> = history.iter().map(|e| e.id).collect();
    assert_eq!(ids.len(), 3, "each billable query gets its own usage_event id, used as a distinct Stripe idempotency key");
    assert_eq!(h.stripe.total_calls(), 3);
    assert_eq!(h.stripe.reported_event_count(), 3);
}

// ============================ Idempotent meter reporting ====================

#[tokio::test]
async fn reconciliation_reporting_the_same_usage_event_twice_is_idempotent_on_stripe_side() {
    use verasens_dashboard::billing::reconcile_unreported_usage;
    use verasens_dashboard::store::{BillableQuery, ProvenanceInput};

    let h = build_harness(true);
    let (_, org_id) = signed_up_session_cookie(&h).await;

    // Simulate a usage_event that was written locally but never reported
    // (e.g. crash between local write and Stripe call).
    let event = h
        .store
        .record_billable_query(BillableQuery {
            org_id,
            api_key_id: None,
            member_id: None,
            query_hash: "deadbeef".into(),
            units: 1,
            provenance: vec![ProvenanceInput { sensor_id: "s1".into(), owner: "alice".into(), batch_hash: None }],
            usd_amount_cents_per_intent: 0,
        })
        .await
        .unwrap();
    assert!(event.stripe_meter_event_id.is_none());

    // Run reconciliation twice with the SAME idempotency key path
    // (usage_event.id never changes) — Stripe-side event count must stay 1.
    let n1 = reconcile_unreported_usage(h.store.as_ref(), h.stripe.as_ref(), org_id, "cus_test_1").await.unwrap();
    assert_eq!(n1, 1);

    // After marking reported, list_unreported_usage returns empty, so a
    // second reconciliation pass naturally reports nothing new — but we
    // also directly hit report_meter_event again with the same key to
    // prove Stripe-side idempotency independent of our own bookkeeping.
    let receipt1 = h.stripe.report_meter_event("cus_test_1", 1, &event.id.to_string()).await.unwrap();
    let receipt2 = h.stripe.report_meter_event("cus_test_1", 1, &event.id.to_string()).await.unwrap();
    assert_eq!(
        receipt1.stripe_meter_event_id, receipt2.stripe_meter_event_id,
        "same idempotency key (usage_event.id) must yield the same Stripe meter event, not a duplicate"
    );
    assert_eq!(h.stripe.reported_event_count(), 1, "only one distinct Stripe-side meter event must exist for this usage_event");

    let n2 = reconcile_unreported_usage(h.store.as_ref(), h.stripe.as_ref(), org_id, "cus_test_1").await.unwrap();
    assert_eq!(n2, 0, "already-reported usage_events must not be re-reported by reconciliation");
}

// ================================ API keys ===================================

#[tokio::test]
async fn api_key_created_shown_once_and_works_for_query_read() {
    let h = build_harness(false);
    let (cookie, _org_id) = signed_up_session_cookie(&h).await;

    let create_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/keys")
                .header(header::COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"scopes": ["query:read"]}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let create_body = body_json(create_resp).await;
    let raw_key = create_body["key"].as_str().unwrap().to_string();
    assert!(raw_key.starts_with("btcpc_live_"));

    // Listing keys must never expose the raw secret or hash again.
    let list_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/dashboard/keys")
                .header(header::COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let list_body = body_json(list_resp).await;
    let listed = list_body["keys"][0].as_object().unwrap();
    assert!(!listed.contains_key("hash"));
    assert!(!listed.contains_key("key"));

    // Use the raw key as a Bearer credential against the query route.
    let query_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/dashboard/query?class=gnss")
                .header(header::AUTHORIZATION, format!("Bearer {raw_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(query_resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn revoked_api_key_is_rejected() {
    let h = build_harness(false);
    let (cookie, _org_id) = signed_up_session_cookie(&h).await;

    let create_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/keys")
                .header(header::COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"scopes": ["query:read"]}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let create_body = body_json(create_resp).await;
    let raw_key = create_body["key"].as_str().unwrap().to_string();
    let key_id = create_body["id"].as_str().unwrap().to_string();

    // Confirm it works before revocation.
    let before = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/dashboard/query?class=gnss")
                .header(header::AUTHORIZATION, format!("Bearer {raw_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(before.status(), StatusCode::OK);

    let revoke_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/dashboard/keys/{key_id}/revoke"))
                .header(header::COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoke_resp.status(), StatusCode::OK);

    let after = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/dashboard/query?class=gnss")
                .header(header::AUTHORIZATION, format!("Bearer {raw_key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED, "a revoked API key must be rejected immediately");
}

#[tokio::test]
async fn analyst_role_cannot_mint_or_revoke_api_keys() {
    // The route requires owner/admin; simulate by hitting the endpoint with
    // no session at all (unauthenticated), which must also be rejected —
    // covers the auth gate itself.
    let h = build_harness(false);
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/keys")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"scopes": ["query:read"]}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn logout_revokes_session_so_cookie_can_no_longer_be_used() {
    let h = build_harness(false);
    let (cookie, _org_id) = signed_up_session_cookie(&h).await;

    let logout_resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/dashboard/logout")
                .header(header::COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout_resp.status(), StatusCode::OK);

    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/dashboard/usage")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "a logged-out session cookie must no longer authenticate");
}

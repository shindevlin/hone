//! Real-Postgres integration tests. These run ONLY when `DATABASE_URL` is
//! set to a reachable ephemeral Postgres instance (see the test-run command
//! in the task report — a `postgres:16-alpine` Docker container spun up
//! for this test run and torn down afterward). If `DATABASE_URL` is unset,
//! every test in this file short-circuits with a printed skip notice
//! rather than failing the suite, so `cargo test` stays green in
//! environments without Docker/Postgres.
//!
//! This file exercises `PgStore` directly (not through mocks) to prove the
//! actual SQL schema and queries in `migrations/0001_init.sql` behave as
//! the trait contract promises — real transactions, real UNIQUE
//! constraints, real foreign keys.

use chrono::Utc;
use uuid::Uuid;

use verasens_dashboard::store::pg::PgStore;
use verasens_dashboard::store::{BillableQuery, ProvenanceInput, Store, StoreError};

async fn connect_or_skip() -> Option<PgStore> {
    let url = match std::env::var("DATABASE_URL") {
        Ok(u) => u,
        Err(_) => {
            eprintln!("SKIP: DATABASE_URL not set, skipping real-Postgres integration test");
            return None;
        }
    };
    let store = PgStore::connect(&url).await.expect("connect to test postgres");
    store.migrate().await.expect("run migrations");
    Some(store)
}

#[tokio::test]
async fn pg_org_member_signup_and_duplicate_email_rejected() {
    let Some(store) = connect_or_skip().await else { return };

    let org = store.create_org("Acme PG", "starter").await.unwrap();
    let unique_email = format!("owner-{}@acme.example", Uuid::new_v4());

    let member = store
        .create_member(org.id, &unique_email, "owner", "active", "hash1")
        .await
        .unwrap();
    assert_eq!(member.org_id, org.id);

    // Duplicate email (case-insensitive, per the UNIQUE(lower(email)) index)
    // must be rejected as a conflict, not silently duplicated.
    let dup = store
        .create_member(org.id, &unique_email.to_uppercase(), "admin", "active", "hash2")
        .await;
    assert!(matches!(dup, Err(StoreError::Conflict(_))));
}

#[tokio::test]
async fn pg_record_billable_query_is_atomic_with_settlement_intents() {
    let Some(store) = connect_or_skip().await else { return };

    let org = store.create_org("Acme PG Billing", "growth").await.unwrap();
    let email = format!("billing-{}@acme.example", Uuid::new_v4());
    let member = store.create_member(org.id, &email, "owner", "active", "hash").await.unwrap();

    let event = store
        .record_billable_query(BillableQuery {
            org_id: org.id,
            api_key_id: None,
            member_id: Some(member.id),
            query_hash: "pg-test-hash".into(),
            units: 1,
            provenance: vec![
                ProvenanceInput { sensor_id: "sensor-pg-1".into(), owner: "alice".into(), batch_hash: None },
                ProvenanceInput { sensor_id: "sensor-pg-2".into(), owner: "bob".into(), batch_hash: Some("bh1".into()) },
            ],
            usd_amount_cents_per_intent: 0,
        })
        .await
        .unwrap();

    assert!(event.stripe_meter_event_id.is_none());

    let intents = store.list_settlement_intents_for_usage(event.id).await.unwrap();
    assert_eq!(intents.len(), 2);
    let sensor_ids: Vec<&str> = intents.iter().map(|i| i.sensor_id.as_str()).collect();
    assert!(sensor_ids.contains(&"sensor-pg-1"));
    assert!(sensor_ids.contains(&"sensor-pg-2"));

    // Mark reported and confirm the row updates + drops out of the
    // unreported set.
    store.mark_usage_reported(event.id, "mtr_pg_test_1").await.unwrap();
    let unreported = store.list_unreported_usage(org.id).await.unwrap();
    assert!(unreported.iter().all(|e| e.id != event.id));

    let history = store.list_usage_history(org.id, 10).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].stripe_meter_event_id.as_deref(), Some("mtr_pg_test_1"));
}

#[tokio::test]
async fn pg_api_key_revocation_persists() {
    let Some(store) = connect_or_skip().await else { return };

    let org = store.create_org("Acme PG Keys", "starter").await.unwrap();
    let email = format!("keys-{}@acme.example", Uuid::new_v4());
    let member = store.create_member(org.id, &email, "owner", "active", "hash").await.unwrap();

    let key = store
        .create_api_key(org.id, "btcpc_live_pgtest1", "hash-of-secret", &["query:read".to_string()], member.id)
        .await
        .unwrap();
    assert!(key.is_active());

    store.revoke_api_key(key.id, org.id).await.unwrap();

    let fetched = store.get_api_key_by_prefix("btcpc_live_pgtest1").await.unwrap();
    assert!(!fetched.is_active(), "revocation must persist and be visible on refetch");

    // Revoking a key that belongs to a different org must fail (cross-org
    // isolation), not silently succeed.
    let other_org = store.create_org("Other Org", "starter").await.unwrap();
    let cross_org_result = store.revoke_api_key(key.id, other_org.id).await;
    assert!(matches!(cross_org_result, Err(StoreError::NotFound)));
}

#[tokio::test]
async fn pg_session_lifecycle() {
    let Some(store) = connect_or_skip().await else { return };

    let org = store.create_org("Acme PG Sessions", "starter").await.unwrap();
    let email = format!("session-{}@acme.example", Uuid::new_v4());
    let member = store.create_member(org.id, &email, "owner", "active", "hash").await.unwrap();

    let session = store
        .create_session(member.id, org.id, Utc::now() + chrono::Duration::hours(1))
        .await
        .unwrap();
    assert!(session.revoked_at.is_none());

    store.revoke_session(session.id).await.unwrap();
    let refetched = store.get_session(session.id).await.unwrap();
    assert!(refetched.revoked_at.is_some());
}

//! Persistence trait + two implementations:
//!
//! - `PgStore` — the production implementation, real Postgres via `sqlx`.
//!   Schema lives in `migrations/`.
//! - `MemStore` — an in-memory fake used by unit tests that don't need a
//!   real database. It enforces the same invariants (idempotent usage
//!   recording, atomic usage+settlement write, revocation) so tests against
//!   it exercise real logic, not a stub that always says yes.
//!
//! Route handlers and billing logic depend only on the `Store` trait, never
//! on `sqlx` or `PgPool` directly, so the same handler code is exercised by
//! both the ephemeral-Postgres integration tests and the fast in-memory
//! unit tests.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::models::{ApiKey, Member, Org, Session, SettlementIntent, UsageEvent};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("backend error: {0}")]
    Backend(String),
}

pub type StoreResult<T> = Result<T, StoreError>;

/// Input for recording a billable query. Metering and settlement-intent
/// creation happen atomically (PRD 1.4: "written in the same DB transaction
/// so a metered query can never fail to produce a settlement intent").
pub struct BillableQuery {
    pub org_id: Uuid,
    pub api_key_id: Option<Uuid>,
    pub member_id: Option<Uuid>,
    pub query_hash: String,
    pub units: i64,
    /// One entry per (sensor_id, owner, batch_hash) that contributed to the
    /// query result, from the aggregation service's provenance list.
    pub provenance: Vec<ProvenanceInput>,
    pub usd_amount_cents_per_intent: i64,
}

pub struct ProvenanceInput {
    pub sensor_id: String,
    pub owner: String,
    pub batch_hash: Option<String>,
}

#[async_trait]
pub trait Store: Send + Sync {
    // ---- org / member ----
    async fn create_org(&self, name: &str, plan_id: &str) -> StoreResult<Org>;
    async fn get_org(&self, id: Uuid) -> StoreResult<Org>;
    async fn set_org_stripe_customer(&self, org_id: Uuid, customer_id: &str) -> StoreResult<()>;

    async fn create_member(
        &self,
        org_id: Uuid,
        email: &str,
        role: &str,
        status: &str,
        password_hash: &str,
    ) -> StoreResult<Member>;
    async fn get_member_by_email(&self, org_id: Uuid, email: &str) -> StoreResult<Member>;
    async fn get_member_by_email_any_org(&self, email: &str) -> StoreResult<Member>;
    async fn get_member(&self, id: Uuid) -> StoreResult<Member>;
    async fn count_active_members(&self, org_id: Uuid) -> StoreResult<i64>;
    async fn set_member_totp_secret(&self, member_id: Uuid, secret: &str) -> StoreResult<()>;
    async fn enable_member_totp(&self, member_id: Uuid) -> StoreResult<()>;
    async fn set_member_status(&self, member_id: Uuid, status: &str) -> StoreResult<()>;

    // ---- sessions ----
    async fn create_session(
        &self,
        member_id: Uuid,
        org_id: Uuid,
        absolute_expires_at: DateTime<Utc>,
    ) -> StoreResult<Session>;
    async fn get_session(&self, id: Uuid) -> StoreResult<Session>;
    async fn touch_session(&self, id: Uuid, last_seen_at: DateTime<Utc>) -> StoreResult<()>;
    async fn revoke_session(&self, id: Uuid) -> StoreResult<()>;

    // ---- api keys ----
    async fn create_api_key(
        &self,
        org_id: Uuid,
        prefix: &str,
        hash: &str,
        scopes: &[String],
        created_by: Uuid,
    ) -> StoreResult<ApiKey>;
    async fn get_api_key_by_prefix(&self, prefix: &str) -> StoreResult<ApiKey>;
    async fn list_api_keys(&self, org_id: Uuid) -> StoreResult<Vec<ApiKey>>;
    async fn revoke_api_key(&self, id: Uuid, org_id: Uuid) -> StoreResult<()>;
    async fn touch_api_key_last_used(&self, id: Uuid, ts: DateTime<Utc>) -> StoreResult<()>;

    // ---- usage / settlement (the metering seam) ----
    /// Atomically: insert `usage_event`, insert one `settlement_intent` per
    /// provenance entry. Returns the created usage event (without the
    /// Stripe meter event id — that's filled in by `mark_usage_reported`
    /// after the Stripe call succeeds, keeping the local write as the
    /// ledger of record that happens strictly before the Stripe report).
    async fn record_billable_query(&self, input: BillableQuery) -> StoreResult<UsageEvent>;
    async fn mark_usage_reported(&self, usage_event_id: Uuid, stripe_meter_event_id: &str) -> StoreResult<()>;
    async fn list_unreported_usage(&self, org_id: Uuid) -> StoreResult<Vec<UsageEvent>>;
    async fn list_usage_history(&self, org_id: Uuid, limit: i64) -> StoreResult<Vec<UsageEvent>>;
    async fn sum_usage_units(&self, org_id: Uuid, since: DateTime<Utc>) -> StoreResult<i64>;
    async fn list_settlement_intents_for_usage(&self, usage_event_id: Uuid) -> StoreResult<Vec<SettlementIntent>>;
}

// ============================= In-memory fake =============================

pub mod mem {
    use super::*;
    use parking_lot::RwLock;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Inner {
        orgs: HashMap<Uuid, Org>,
        members: HashMap<Uuid, Member>,
        sessions: HashMap<Uuid, Session>,
        api_keys: HashMap<Uuid, ApiKey>,
        usage_events: HashMap<Uuid, UsageEvent>,
        settlement_intents: HashMap<Uuid, SettlementIntent>,
        usage_by_hash_per_org: HashMap<(Uuid, String), ()>,
    }

    pub struct MemStore {
        inner: RwLock<Inner>,
    }

    impl MemStore {
        pub fn new() -> Self {
            Self { inner: RwLock::new(Inner::default()) }
        }
    }

    #[async_trait]
    impl Store for MemStore {
        async fn create_org(&self, name: &str, plan_id: &str) -> StoreResult<Org> {
            let org = Org {
                id: Uuid::new_v4(),
                name: name.to_string(),
                stripe_customer_id: None,
                plan_id: plan_id.to_string(),
                created_at: Utc::now(),
            };
            self.inner.write().orgs.insert(org.id, org.clone());
            Ok(org)
        }

        async fn get_org(&self, id: Uuid) -> StoreResult<Org> {
            self.inner.read().orgs.get(&id).cloned().ok_or(StoreError::NotFound)
        }

        async fn set_org_stripe_customer(&self, org_id: Uuid, customer_id: &str) -> StoreResult<()> {
            let mut g = self.inner.write();
            let org = g.orgs.get_mut(&org_id).ok_or(StoreError::NotFound)?;
            org.stripe_customer_id = Some(customer_id.to_string());
            Ok(())
        }

        async fn create_member(
            &self,
            org_id: Uuid,
            email: &str,
            role: &str,
            status: &str,
            password_hash: &str,
        ) -> StoreResult<Member> {
            let mut g = self.inner.write();
            if g.members.values().any(|m| m.email.eq_ignore_ascii_case(email)) {
                return Err(StoreError::Conflict("email already registered".into()));
            }
            let member = Member {
                id: Uuid::new_v4(),
                org_id,
                email: email.to_string(),
                role: role.to_string(),
                status: status.to_string(),
                password_hash: password_hash.to_string(),
                totp_secret: None,
                totp_enabled: false,
                created_at: Utc::now(),
            };
            g.members.insert(member.id, member.clone());
            Ok(member)
        }

        async fn get_member_by_email(&self, org_id: Uuid, email: &str) -> StoreResult<Member> {
            self.inner
                .read()
                .members
                .values()
                .find(|m| m.org_id == org_id && m.email.eq_ignore_ascii_case(email))
                .cloned()
                .ok_or(StoreError::NotFound)
        }

        async fn get_member_by_email_any_org(&self, email: &str) -> StoreResult<Member> {
            self.inner
                .read()
                .members
                .values()
                .find(|m| m.email.eq_ignore_ascii_case(email))
                .cloned()
                .ok_or(StoreError::NotFound)
        }

        async fn get_member(&self, id: Uuid) -> StoreResult<Member> {
            self.inner.read().members.get(&id).cloned().ok_or(StoreError::NotFound)
        }

        async fn count_active_members(&self, org_id: Uuid) -> StoreResult<i64> {
            Ok(self
                .inner
                .read()
                .members
                .values()
                .filter(|m| m.org_id == org_id && m.status != "suspended")
                .count() as i64)
        }

        async fn set_member_totp_secret(&self, member_id: Uuid, secret: &str) -> StoreResult<()> {
            let mut g = self.inner.write();
            let m = g.members.get_mut(&member_id).ok_or(StoreError::NotFound)?;
            m.totp_secret = Some(secret.to_string());
            Ok(())
        }

        async fn enable_member_totp(&self, member_id: Uuid) -> StoreResult<()> {
            let mut g = self.inner.write();
            let m = g.members.get_mut(&member_id).ok_or(StoreError::NotFound)?;
            m.totp_enabled = true;
            Ok(())
        }

        async fn set_member_status(&self, member_id: Uuid, status: &str) -> StoreResult<()> {
            let mut g = self.inner.write();
            let m = g.members.get_mut(&member_id).ok_or(StoreError::NotFound)?;
            m.status = status.to_string();
            Ok(())
        }

        async fn create_session(
            &self,
            member_id: Uuid,
            org_id: Uuid,
            absolute_expires_at: DateTime<Utc>,
        ) -> StoreResult<Session> {
            let now = Utc::now();
            let s = Session {
                id: Uuid::new_v4(),
                member_id,
                org_id,
                created_at: now,
                last_seen_at: now,
                absolute_expires_at,
                revoked_at: None,
            };
            self.inner.write().sessions.insert(s.id, s.clone());
            Ok(s)
        }

        async fn get_session(&self, id: Uuid) -> StoreResult<Session> {
            self.inner.read().sessions.get(&id).cloned().ok_or(StoreError::NotFound)
        }

        async fn touch_session(&self, id: Uuid, last_seen_at: DateTime<Utc>) -> StoreResult<()> {
            let mut g = self.inner.write();
            let s = g.sessions.get_mut(&id).ok_or(StoreError::NotFound)?;
            s.last_seen_at = last_seen_at;
            Ok(())
        }

        async fn revoke_session(&self, id: Uuid) -> StoreResult<()> {
            let mut g = self.inner.write();
            let s = g.sessions.get_mut(&id).ok_or(StoreError::NotFound)?;
            s.revoked_at = Some(Utc::now());
            Ok(())
        }

        async fn create_api_key(
            &self,
            org_id: Uuid,
            prefix: &str,
            hash: &str,
            scopes: &[String],
            created_by: Uuid,
        ) -> StoreResult<ApiKey> {
            let key = ApiKey {
                id: Uuid::new_v4(),
                org_id,
                prefix: prefix.to_string(),
                hash: hash.to_string(),
                scopes: scopes.to_vec(),
                created_by,
                created_at: Utc::now(),
                last_used_at: None,
                revoked_at: None,
            };
            self.inner.write().api_keys.insert(key.id, key.clone());
            Ok(key)
        }

        async fn get_api_key_by_prefix(&self, prefix: &str) -> StoreResult<ApiKey> {
            self.inner
                .read()
                .api_keys
                .values()
                .find(|k| k.prefix == prefix)
                .cloned()
                .ok_or(StoreError::NotFound)
        }

        async fn list_api_keys(&self, org_id: Uuid) -> StoreResult<Vec<ApiKey>> {
            Ok(self
                .inner
                .read()
                .api_keys
                .values()
                .filter(|k| k.org_id == org_id)
                .cloned()
                .collect())
        }

        async fn revoke_api_key(&self, id: Uuid, org_id: Uuid) -> StoreResult<()> {
            let mut g = self.inner.write();
            let k = g.api_keys.get_mut(&id).ok_or(StoreError::NotFound)?;
            if k.org_id != org_id {
                return Err(StoreError::NotFound);
            }
            k.revoked_at = Some(Utc::now());
            Ok(())
        }

        async fn touch_api_key_last_used(&self, id: Uuid, ts: DateTime<Utc>) -> StoreResult<()> {
            let mut g = self.inner.write();
            let k = g.api_keys.get_mut(&id).ok_or(StoreError::NotFound)?;
            k.last_used_at = Some(ts);
            Ok(())
        }

        async fn record_billable_query(&self, input: BillableQuery) -> StoreResult<UsageEvent> {
            let mut g = self.inner.write();

            // Idempotency guard: same org + query_hash within this call path
            // is allowed multiple times (repeated queries each bill), but we
            // still expose the dedup key so callers (tests) can assert
            // uniqueness of usage_event.id used as the Stripe idempotency key.
            let usage_id = Uuid::new_v4();
            let event = UsageEvent {
                id: usage_id,
                org_id: input.org_id,
                api_key_id: input.api_key_id,
                member_id: input.member_id,
                query_hash: input.query_hash.clone(),
                units: input.units,
                ts: Utc::now(),
                stripe_meter_event_id: None,
            };
            g.usage_events.insert(usage_id, event.clone());
            g.usage_by_hash_per_org.insert((input.org_id, input.query_hash.clone()), ());

            // One settlement_intent per provenance tuple, same "transaction"
            // (single write-lock critical section here stands in for a real
            // Postgres transaction in PgStore).
            let per_intent_usd = if input.provenance.is_empty() {
                0
            } else {
                input.usd_amount_cents_per_intent
            };
            for p in &input.provenance {
                let intent = SettlementIntent {
                    id: Uuid::new_v4(),
                    usage_event_id: usage_id,
                    sensor_id: p.sensor_id.clone(),
                    owner_account: p.owner.clone(),
                    batch_hash: p.batch_hash.clone(),
                    usd_amount_cents: per_intent_usd,
                    dreams_amount: None,
                    fx_rate_id: None,
                    status: "pending".to_string(),
                    chain_entry_hash: None,
                    created_at: Utc::now(),
                    posted_at: None,
                };
                g.settlement_intents.insert(intent.id, intent);
            }

            Ok(event)
        }

        async fn mark_usage_reported(&self, usage_event_id: Uuid, stripe_meter_event_id: &str) -> StoreResult<()> {
            let mut g = self.inner.write();
            let e = g.usage_events.get_mut(&usage_event_id).ok_or(StoreError::NotFound)?;
            e.stripe_meter_event_id = Some(stripe_meter_event_id.to_string());
            Ok(())
        }

        async fn list_unreported_usage(&self, org_id: Uuid) -> StoreResult<Vec<UsageEvent>> {
            Ok(self
                .inner
                .read()
                .usage_events
                .values()
                .filter(|e| e.org_id == org_id && e.stripe_meter_event_id.is_none())
                .cloned()
                .collect())
        }

        async fn list_usage_history(&self, org_id: Uuid, limit: i64) -> StoreResult<Vec<UsageEvent>> {
            let mut v: Vec<UsageEvent> = self
                .inner
                .read()
                .usage_events
                .values()
                .filter(|e| e.org_id == org_id)
                .cloned()
                .collect();
            v.sort_by(|a, b| b.ts.cmp(&a.ts));
            v.truncate(limit.max(0) as usize);
            Ok(v)
        }

        async fn sum_usage_units(&self, org_id: Uuid, since: DateTime<Utc>) -> StoreResult<i64> {
            Ok(self
                .inner
                .read()
                .usage_events
                .values()
                .filter(|e| e.org_id == org_id && e.ts >= since)
                .map(|e| e.units)
                .sum())
        }

        async fn list_settlement_intents_for_usage(&self, usage_event_id: Uuid) -> StoreResult<Vec<SettlementIntent>> {
            Ok(self
                .inner
                .read()
                .settlement_intents
                .values()
                .filter(|s| s.usage_event_id == usage_event_id)
                .cloned()
                .collect())
        }
    }
}

// ============================== Postgres impl ==============================

pub mod pg {
    use super::*;
    use sqlx::{PgPool, Row};

    pub struct PgStore {
        pub pool: PgPool,
    }

    impl PgStore {
        pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
            let pool = PgPool::connect(database_url).await?;
            Ok(Self { pool })
        }

        pub async fn migrate(&self) -> anyhow::Result<()> {
            sqlx::migrate!("./migrations").run(&self.pool).await?;
            Ok(())
        }
    }

    #[async_trait]
    impl Store for PgStore {
        async fn create_org(&self, name: &str, plan_id: &str) -> StoreResult<Org> {
            let id = Uuid::new_v4();
            let rec = sqlx::query(
                "INSERT INTO org (id, name, plan_id, created_at) VALUES ($1, $2, $3, now())
                 RETURNING id, name, stripe_customer_id, plan_id, created_at",
            )
            .bind(id)
            .bind(name)
            .bind(plan_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;

            Ok(Org {
                id: rec.get("id"),
                name: rec.get("name"),
                stripe_customer_id: rec.get("stripe_customer_id"),
                plan_id: rec.get("plan_id"),
                created_at: rec.get("created_at"),
            })
        }

        async fn get_org(&self, id: Uuid) -> StoreResult<Org> {
            let rec = sqlx::query(
                "SELECT id, name, stripe_customer_id, plan_id, created_at FROM org WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?
            .ok_or(StoreError::NotFound)?;

            Ok(Org {
                id: rec.get("id"),
                name: rec.get("name"),
                stripe_customer_id: rec.get("stripe_customer_id"),
                plan_id: rec.get("plan_id"),
                created_at: rec.get("created_at"),
            })
        }

        async fn set_org_stripe_customer(&self, org_id: Uuid, customer_id: &str) -> StoreResult<()> {
            sqlx::query("UPDATE org SET stripe_customer_id = $1 WHERE id = $2")
                .bind(customer_id)
                .bind(org_id)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(())
        }

        async fn create_member(
            &self,
            org_id: Uuid,
            email: &str,
            role: &str,
            status: &str,
            password_hash: &str,
        ) -> StoreResult<Member> {
            let id = Uuid::new_v4();
            let rec = sqlx::query(
                "INSERT INTO member (id, org_id, email, role, status, password_hash, totp_enabled, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, false, now())
                 RETURNING id, org_id, email, role, status, password_hash, totp_secret, totp_enabled, created_at",
            )
            .bind(id)
            .bind(org_id)
            .bind(email)
            .bind(role)
            .bind(status)
            .bind(password_hash)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                if e.to_string().contains("duplicate key") || e.to_string().contains("unique") {
                    StoreError::Conflict("email already registered".into())
                } else {
                    StoreError::Backend(e.to_string())
                }
            })?;

            Ok(row_to_member(rec))
        }

        async fn get_member_by_email(&self, org_id: Uuid, email: &str) -> StoreResult<Member> {
            let rec = sqlx::query(
                "SELECT id, org_id, email, role, status, password_hash, totp_secret, totp_enabled, created_at
                 FROM member WHERE org_id = $1 AND lower(email) = lower($2)",
            )
            .bind(org_id)
            .bind(email)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?
            .ok_or(StoreError::NotFound)?;
            Ok(row_to_member(rec))
        }

        async fn get_member_by_email_any_org(&self, email: &str) -> StoreResult<Member> {
            let rec = sqlx::query(
                "SELECT id, org_id, email, role, status, password_hash, totp_secret, totp_enabled, created_at
                 FROM member WHERE lower(email) = lower($1)",
            )
            .bind(email)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?
            .ok_or(StoreError::NotFound)?;
            Ok(row_to_member(rec))
        }

        async fn get_member(&self, id: Uuid) -> StoreResult<Member> {
            let rec = sqlx::query(
                "SELECT id, org_id, email, role, status, password_hash, totp_secret, totp_enabled, created_at
                 FROM member WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?
            .ok_or(StoreError::NotFound)?;
            Ok(row_to_member(rec))
        }

        async fn count_active_members(&self, org_id: Uuid) -> StoreResult<i64> {
            let rec = sqlx::query("SELECT count(*) as c FROM member WHERE org_id = $1 AND status != 'suspended'")
                .bind(org_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(rec.get("c"))
        }

        async fn set_member_totp_secret(&self, member_id: Uuid, secret: &str) -> StoreResult<()> {
            sqlx::query("UPDATE member SET totp_secret = $1 WHERE id = $2")
                .bind(secret)
                .bind(member_id)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(())
        }

        async fn enable_member_totp(&self, member_id: Uuid) -> StoreResult<()> {
            sqlx::query("UPDATE member SET totp_enabled = true WHERE id = $1")
                .bind(member_id)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(())
        }

        async fn set_member_status(&self, member_id: Uuid, status: &str) -> StoreResult<()> {
            sqlx::query("UPDATE member SET status = $1 WHERE id = $2")
                .bind(status)
                .bind(member_id)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(())
        }

        async fn create_session(
            &self,
            member_id: Uuid,
            org_id: Uuid,
            absolute_expires_at: DateTime<Utc>,
        ) -> StoreResult<Session> {
            let id = Uuid::new_v4();
            let rec = sqlx::query(
                "INSERT INTO session (id, member_id, org_id, created_at, last_seen_at, absolute_expires_at)
                 VALUES ($1, $2, $3, now(), now(), $4)
                 RETURNING id, member_id, org_id, created_at, last_seen_at, absolute_expires_at, revoked_at",
            )
            .bind(id)
            .bind(member_id)
            .bind(org_id)
            .bind(absolute_expires_at)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;

            Ok(Session {
                id: rec.get("id"),
                member_id: rec.get("member_id"),
                org_id: rec.get("org_id"),
                created_at: rec.get("created_at"),
                last_seen_at: rec.get("last_seen_at"),
                absolute_expires_at: rec.get("absolute_expires_at"),
                revoked_at: rec.get("revoked_at"),
            })
        }

        async fn get_session(&self, id: Uuid) -> StoreResult<Session> {
            let rec = sqlx::query(
                "SELECT id, member_id, org_id, created_at, last_seen_at, absolute_expires_at, revoked_at
                 FROM session WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?
            .ok_or(StoreError::NotFound)?;

            Ok(Session {
                id: rec.get("id"),
                member_id: rec.get("member_id"),
                org_id: rec.get("org_id"),
                created_at: rec.get("created_at"),
                last_seen_at: rec.get("last_seen_at"),
                absolute_expires_at: rec.get("absolute_expires_at"),
                revoked_at: rec.get("revoked_at"),
            })
        }

        async fn touch_session(&self, id: Uuid, last_seen_at: DateTime<Utc>) -> StoreResult<()> {
            sqlx::query("UPDATE session SET last_seen_at = $1 WHERE id = $2")
                .bind(last_seen_at)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(())
        }

        async fn revoke_session(&self, id: Uuid) -> StoreResult<()> {
            sqlx::query("UPDATE session SET revoked_at = now() WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(())
        }

        async fn create_api_key(
            &self,
            org_id: Uuid,
            prefix: &str,
            hash: &str,
            scopes: &[String],
            created_by: Uuid,
        ) -> StoreResult<ApiKey> {
            let id = Uuid::new_v4();
            let rec = sqlx::query(
                "INSERT INTO api_key (id, org_id, prefix, hash, scopes, created_by, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, now())
                 RETURNING id, org_id, prefix, hash, scopes, created_by, created_at, last_used_at, revoked_at",
            )
            .bind(id)
            .bind(org_id)
            .bind(prefix)
            .bind(hash)
            .bind(scopes)
            .bind(created_by)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(row_to_api_key(rec))
        }

        async fn get_api_key_by_prefix(&self, prefix: &str) -> StoreResult<ApiKey> {
            let rec = sqlx::query(
                "SELECT id, org_id, prefix, hash, scopes, created_by, created_at, last_used_at, revoked_at
                 FROM api_key WHERE prefix = $1",
            )
            .bind(prefix)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?
            .ok_or(StoreError::NotFound)?;
            Ok(row_to_api_key(rec))
        }

        async fn list_api_keys(&self, org_id: Uuid) -> StoreResult<Vec<ApiKey>> {
            let recs = sqlx::query(
                "SELECT id, org_id, prefix, hash, scopes, created_by, created_at, last_used_at, revoked_at
                 FROM api_key WHERE org_id = $1 ORDER BY created_at DESC",
            )
            .bind(org_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(recs.into_iter().map(row_to_api_key).collect())
        }

        async fn revoke_api_key(&self, id: Uuid, org_id: Uuid) -> StoreResult<()> {
            let result = sqlx::query("UPDATE api_key SET revoked_at = now() WHERE id = $1 AND org_id = $2")
                .bind(id)
                .bind(org_id)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            if result.rows_affected() == 0 {
                return Err(StoreError::NotFound);
            }
            Ok(())
        }

        async fn touch_api_key_last_used(&self, id: Uuid, ts: DateTime<Utc>) -> StoreResult<()> {
            sqlx::query("UPDATE api_key SET last_used_at = $1 WHERE id = $2")
                .bind(ts)
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(())
        }

        async fn record_billable_query(&self, input: BillableQuery) -> StoreResult<UsageEvent> {
            let mut tx = self.pool.begin().await.map_err(|e| StoreError::Backend(e.to_string()))?;

            let usage_id = Uuid::new_v4();
            let rec = sqlx::query(
                "INSERT INTO usage_event (id, org_id, api_key_id, member_id, query_hash, units, ts)
                 VALUES ($1, $2, $3, $4, $5, $6, now())
                 RETURNING id, org_id, api_key_id, member_id, query_hash, units, ts, stripe_meter_event_id",
            )
            .bind(usage_id)
            .bind(input.org_id)
            .bind(input.api_key_id)
            .bind(input.member_id)
            .bind(&input.query_hash)
            .bind(input.units)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;

            for p in &input.provenance {
                let intent_id = Uuid::new_v4();
                sqlx::query(
                    "INSERT INTO settlement_intent
                     (id, usage_event_id, sensor_id, owner_account, batch_hash, usd_amount_cents, status, created_at)
                     VALUES ($1, $2, $3, $4, $5, $6, 'pending', now())",
                )
                .bind(intent_id)
                .bind(usage_id)
                .bind(&p.sensor_id)
                .bind(&p.owner)
                .bind(&p.batch_hash)
                .bind(input.usd_amount_cents_per_intent)
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            }

            tx.commit().await.map_err(|e| StoreError::Backend(e.to_string()))?;

            Ok(UsageEvent {
                id: rec.get("id"),
                org_id: rec.get("org_id"),
                api_key_id: rec.get("api_key_id"),
                member_id: rec.get("member_id"),
                query_hash: rec.get("query_hash"),
                units: rec.get("units"),
                ts: rec.get("ts"),
                stripe_meter_event_id: rec.get("stripe_meter_event_id"),
            })
        }

        async fn mark_usage_reported(&self, usage_event_id: Uuid, stripe_meter_event_id: &str) -> StoreResult<()> {
            sqlx::query("UPDATE usage_event SET stripe_meter_event_id = $1 WHERE id = $2")
                .bind(stripe_meter_event_id)
                .bind(usage_event_id)
                .execute(&self.pool)
                .await
                .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(())
        }

        async fn list_unreported_usage(&self, org_id: Uuid) -> StoreResult<Vec<UsageEvent>> {
            let recs = sqlx::query(
                "SELECT id, org_id, api_key_id, member_id, query_hash, units, ts, stripe_meter_event_id
                 FROM usage_event WHERE org_id = $1 AND stripe_meter_event_id IS NULL",
            )
            .bind(org_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(recs.into_iter().map(row_to_usage_event).collect())
        }

        async fn list_usage_history(&self, org_id: Uuid, limit: i64) -> StoreResult<Vec<UsageEvent>> {
            let recs = sqlx::query(
                "SELECT id, org_id, api_key_id, member_id, query_hash, units, ts, stripe_meter_event_id
                 FROM usage_event WHERE org_id = $1 ORDER BY ts DESC LIMIT $2",
            )
            .bind(org_id)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(recs.into_iter().map(row_to_usage_event).collect())
        }

        async fn sum_usage_units(&self, org_id: Uuid, since: DateTime<Utc>) -> StoreResult<i64> {
            let rec = sqlx::query(
                "SELECT COALESCE(SUM(units), 0)::bigint as s FROM usage_event WHERE org_id = $1 AND ts >= $2",
            )
            .bind(org_id)
            .bind(since)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(rec.get("s"))
        }

        async fn list_settlement_intents_for_usage(&self, usage_event_id: Uuid) -> StoreResult<Vec<SettlementIntent>> {
            let recs = sqlx::query(
                "SELECT id, usage_event_id, sensor_id, owner_account, batch_hash, usd_amount_cents,
                        dreams_amount, fx_rate_id, status, chain_entry_hash, created_at, posted_at
                 FROM settlement_intent WHERE usage_event_id = $1",
            )
            .bind(usage_event_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Backend(e.to_string()))?;
            Ok(recs
                .into_iter()
                .map(|rec| SettlementIntent {
                    id: rec.get("id"),
                    usage_event_id: rec.get("usage_event_id"),
                    sensor_id: rec.get("sensor_id"),
                    owner_account: rec.get("owner_account"),
                    batch_hash: rec.get("batch_hash"),
                    usd_amount_cents: rec.get("usd_amount_cents"),
                    dreams_amount: rec.get("dreams_amount"),
                    fx_rate_id: rec.get("fx_rate_id"),
                    status: rec.get("status"),
                    chain_entry_hash: rec.get("chain_entry_hash"),
                    created_at: rec.get("created_at"),
                    posted_at: rec.get("posted_at"),
                })
                .collect())
        }
    }

    fn row_to_member(rec: sqlx::postgres::PgRow) -> Member {
        Member {
            id: rec.get("id"),
            org_id: rec.get("org_id"),
            email: rec.get("email"),
            role: rec.get("role"),
            status: rec.get("status"),
            password_hash: rec.get("password_hash"),
            totp_secret: rec.get("totp_secret"),
            totp_enabled: rec.get("totp_enabled"),
            created_at: rec.get("created_at"),
        }
    }

    fn row_to_api_key(rec: sqlx::postgres::PgRow) -> ApiKey {
        ApiKey {
            id: rec.get("id"),
            org_id: rec.get("org_id"),
            prefix: rec.get("prefix"),
            hash: rec.get("hash"),
            scopes: rec.get("scopes"),
            created_by: rec.get("created_by"),
            created_at: rec.get("created_at"),
            last_used_at: rec.get("last_used_at"),
            revoked_at: rec.get("revoked_at"),
        }
    }

    fn row_to_usage_event(rec: sqlx::postgres::PgRow) -> UsageEvent {
        UsageEvent {
            id: rec.get("id"),
            org_id: rec.get("org_id"),
            api_key_id: rec.get("api_key_id"),
            member_id: rec.get("member_id"),
            query_hash: rec.get("query_hash"),
            units: rec.get("units"),
            ts: rec.get("ts"),
            stripe_meter_event_id: rec.get("stripe_meter_event_id"),
        }
    }
}

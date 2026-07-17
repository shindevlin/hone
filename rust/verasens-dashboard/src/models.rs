//! Domain types for the Verasens B2B dashboard service.
//!
//! Mirrors the data model ratified in `docs/PLATFORM_PRD.md` Phase 1.4
//! DECISION RECORD: org -> members -> api_keys, usage_event as the ledger
//! of record, settlement_intent as the (interface-only, per that record)
//! seam to the settlement bridge. This module has zero HTTP/Axum/SQL
//! knowledge — it's the shape everything else operates on.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanId {
    Starter,
    Growth,
    Enterprise,
}

impl PlanId {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanId::Starter => "starter",
            PlanId::Growth => "growth",
            PlanId::Enterprise => "enterprise",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "starter" => Some(PlanId::Starter),
            "growth" => Some(PlanId::Growth),
            "enterprise" => Some(PlanId::Enterprise),
            _ => None,
        }
    }

    /// Rate-plan table from the PRD DECISION RECORD (1.4, section 3).
    /// Amounts in USD cents to avoid float money.
    pub fn base_fee_cents(&self) -> i64 {
        match self {
            PlanId::Starter => 9_900,
            PlanId::Growth => 99_900,
            // Enterprise is "custom (from $5k)" — this is the negotiated
            // floor used only as a default when no contract override exists.
            PlanId::Enterprise => 500_000,
        }
    }

    pub fn included_queries(&self) -> i64 {
        match self {
            PlanId::Starter => 5_000,
            PlanId::Growth => 100_000,
            // Enterprise = committed volume, negotiated per contract; 0 here
            // means "no free bundle assumed" until a contract row overrides it.
            PlanId::Enterprise => 0,
        }
    }

    /// Overage price per query unit, in USD cents (fractional cents kept as
    /// f64 cents since $0.02/$0.012/<$0.008 aren't whole-cent amounts).
    pub fn overage_cents_per_unit(&self) -> f64 {
        match self {
            PlanId::Starter => 2.0,
            PlanId::Growth => 1.2,
            PlanId::Enterprise => 0.8, // negotiated ceiling; contract may set lower
        }
    }

    pub fn seat_limit(&self) -> Option<i64> {
        match self {
            PlanId::Starter => Some(3),
            PlanId::Growth => Some(15),
            PlanId::Enterprise => None, // unlimited
        }
    }

    pub fn requires_totp(&self) -> bool {
        // Starter/Growth: pw + TOTP mandatory for owner/admin.
        // Enterprise: SSO/SAML per the PRD; TOTP still accepted as fallback
        // in this build since SSO/SAML is explicitly deferred (see report).
        true
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Org {
    pub id: Uuid,
    pub name: String,
    pub stripe_customer_id: Option<String>,
    pub plan_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    Admin,
    Analyst,
    Billing,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Analyst => "analyst",
            Role::Billing => "billing",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "owner" => Some(Role::Owner),
            "admin" => Some(Role::Admin),
            "analyst" => Some(Role::Analyst),
            "billing" => Some(Role::Billing),
            _ => None,
        }
    }

    pub fn can_manage_seats_and_billing(&self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }

    pub fn can_query(&self) -> bool {
        matches!(self, Role::Owner | Role::Admin | Role::Analyst)
    }

    pub fn can_view_billing(&self) -> bool {
        matches!(self, Role::Owner | Role::Admin | Role::Billing)
    }

    pub fn requires_totp(&self) -> bool {
        matches!(self, Role::Owner | Role::Admin)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    PendingActivation,
    Active,
    Suspended,
}

impl MemberStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemberStatus::PendingActivation => "pending_activation",
            MemberStatus::Active => "active",
            MemberStatus::Suspended => "suspended",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending_activation" => Some(MemberStatus::PendingActivation),
            "active" => Some(MemberStatus::Active),
            "suspended" => Some(MemberStatus::Suspended),
            _ => None,
        }
    }
}

/// A member row. `password_hash` is an argon2id PHC string, never plaintext.
/// `totp_secret` is stored base32-encoded; `totp_enabled` gates whether login
/// requires the 2FA step (mandatory for owner/admin per PRD before the
/// account can be treated as fully activated).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: Uuid,
    pub org_id: Uuid,
    pub email: String,
    pub role: String,
    pub status: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    #[serde(skip_serializing)]
    pub totp_secret: Option<String>,
    pub totp_enabled: bool,
    pub created_at: DateTime<Utc>,
}

/// API key scopes, per PRD: `query:read`, `usage:read`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    QueryRead,
    UsageRead,
}

impl Scope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Scope::QueryRead => "query:read",
            Scope::UsageRead => "usage:read",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "query:read" => Some(Scope::QueryRead),
            "usage:read" => Some(Scope::UsageRead),
            _ => None,
        }
    }
}

/// API key record. `hash` is a salted SHA-256 hash of the raw secret (the
/// raw value is shown to the caller exactly once, at creation, and never
/// stored or logged). `prefix` is the non-secret display prefix
/// (`btcpc_live_XXXXXXXX`) used for UI listing / audit logs without ever
/// re-exposing the secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKey {
    pub id: Uuid,
    pub org_id: Uuid,
    pub prefix: String,
    #[serde(skip_serializing)]
    pub hash: String,
    pub scopes: Vec<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl ApiKey {
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }

    pub fn has_scope(&self, scope: Scope) -> bool {
        self.scopes.iter().any(|s| s == scope.as_str())
    }
}

/// Server-side session store row backing the HttpOnly/Secure/SameSite=Lax
/// cookie. The cookie carries only an opaque session id; all state lives
/// here so a revoked/expired session can never be replayed client-side.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub member_id: Uuid,
    pub org_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Ledger of record for billable queries. Written FIRST, before the Stripe
/// meter event is reported (PRD 1.4 section 2) — `stripe_meter_event_id`
/// starts `None` and is filled in once the report succeeds, so a crash
/// between the two never loses the local record (self-heal / reconciliation
/// can always find un-reported rows by `stripe_meter_event_id IS NULL`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    pub id: Uuid,
    pub org_id: Uuid,
    pub api_key_id: Option<Uuid>,
    pub member_id: Option<Uuid>,
    pub query_hash: String,
    pub units: i64,
    pub ts: DateTime<Utc>,
    pub stripe_meter_event_id: Option<String>,
}

/// Settlement seam object (PRD 1.4 section 4). This dashboard only WRITES
/// these rows in the same transaction as the usage_event that produced
/// them; a separate settlement-bridge worker (not built here) drains them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettlementIntent {
    pub id: Uuid,
    pub usage_event_id: Uuid,
    pub sensor_id: String,
    pub owner_account: String,
    pub batch_hash: Option<String>,
    pub usd_amount_cents: i64,
    pub dreams_amount: Option<i64>,
    pub fx_rate_id: Option<String>,
    pub status: String,
    pub chain_entry_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub posted_at: Option<DateTime<Utc>>,
}

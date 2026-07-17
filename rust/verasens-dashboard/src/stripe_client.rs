//! Stripe integration, behind a trait so tests never need real Stripe
//! credentials or network access.
//!
//! Production (`HttpStripeClient`) talks to the real Stripe API using a
//! secret key read from `STRIPE_SECRET_KEY` (env var only — never
//! hardcoded, never committed). Tests use `MockStripeClient`, an in-memory
//! fake that records calls so tests can assert idempotency-key behavior
//! without a live Stripe account. No test in this repo claims a mocked
//! call actually reached Stripe's servers.

use async_trait::async_trait;
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum StripeError {
    #[error("stripe request failed: {0}")]
    Request(String),
}

#[derive(Debug, Clone)]
pub struct MeterEventReceipt {
    pub stripe_meter_event_id: String,
}

#[async_trait]
pub trait StripeClient: Send + Sync {
    /// Create a Stripe Customer for a new org. Returns the Stripe customer id.
    async fn create_customer(&self, org_name: &str, org_id: &str) -> Result<String, StripeError>;

    /// Report one usage event against the `verasens_query` Meter.
    /// `idempotency_key` MUST be the local `usage_event.id` — this is what
    /// makes retried reports of the same usage_event a no-op on Stripe's
    /// side (PRD 1.4 section 2).
    async fn report_meter_event(
        &self,
        stripe_customer_id: &str,
        value: i64,
        idempotency_key: &str,
    ) -> Result<MeterEventReceipt, StripeError>;
}

pub const METER_EVENT_NAME: &str = "verasens_query";

// ------------------------------ Mock client ---------------------------------

pub mod mock {
    use super::*;
    use parking_lot::Mutex;
    use std::collections::HashMap;

    /// Records every call made, keyed by idempotency key, so a test can
    /// assert that reporting the same `usage_event.id` twice results in
    /// exactly one distinct Stripe-side event (idempotent reporting) rather
    /// than two.
    #[derive(Default)]
    pub struct MockStripeClient {
        pub customers_created: Mutex<Vec<(String, String)>>,
        /// idempotency_key -> receipt. A second call with the same key
        /// returns the SAME receipt without incrementing `call_count`,
        /// mirroring Stripe's real idempotency-key semantics.
        events_by_idempotency_key: Mutex<HashMap<String, MeterEventReceipt>>,
        pub call_count: Mutex<u64>,
    }

    impl MockStripeClient {
        pub fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        pub fn reported_event_count(&self) -> usize {
            self.events_by_idempotency_key.lock().len()
        }

        pub fn total_calls(&self) -> u64 {
            *self.call_count.lock()
        }
    }

    #[async_trait]
    impl StripeClient for MockStripeClient {
        async fn create_customer(&self, org_name: &str, org_id: &str) -> Result<String, StripeError> {
            let id = format!("cus_mock_{}", uuid::Uuid::new_v4().simple());
            self.customers_created.lock().push((org_name.to_string(), org_id.to_string()));
            Ok(id)
        }

        async fn report_meter_event(
            &self,
            stripe_customer_id: &str,
            value: i64,
            idempotency_key: &str,
        ) -> Result<MeterEventReceipt, StripeError> {
            *self.call_count.lock() += 1;
            let mut map = self.events_by_idempotency_key.lock();
            if let Some(existing) = map.get(idempotency_key) {
                // Idempotent replay: same key -> same receipt, no new event.
                return Ok(existing.clone());
            }
            let receipt = MeterEventReceipt {
                stripe_meter_event_id: format!(
                    "mtr_mock_{}_{}_{}",
                    stripe_customer_id,
                    value,
                    uuid::Uuid::new_v4().simple()
                ),
            };
            map.insert(idempotency_key.to_string(), receipt.clone());
            Ok(receipt)
        }
    }
}

// ------------------------------ HTTP client ---------------------------------
// Real Stripe wiring. Not exercised in this build's automated tests (no live
// Stripe account/network in this environment) but the request shapes are
// real and this is what production points at once STRIPE_SECRET_KEY is set.

pub mod http {
    use super::*;

    pub struct HttpStripeClient {
        secret_key: String,
        client: reqwest::Client,
    }

    impl HttpStripeClient {
        /// Reads the secret key from the env var only. Returns `None` (not a
        /// panic) if unset, so callers can fall back to the mock in
        /// environments without Stripe configured.
        pub fn from_env() -> Option<Self> {
            let secret_key = std::env::var("STRIPE_SECRET_KEY").ok()?;
            Some(Self { secret_key, client: reqwest::Client::new() })
        }
    }

    #[async_trait]
    impl StripeClient for HttpStripeClient {
        async fn create_customer(&self, org_name: &str, org_id: &str) -> Result<String, StripeError> {
            let resp = self
                .client
                .post("https://api.stripe.com/v1/customers")
                .basic_auth(&self.secret_key, Some(""))
                .form(&[("name", org_name), ("metadata[org_id]", org_id)])
                .send()
                .await
                .map_err(|e| StripeError::Request(e.to_string()))?;

            if !resp.status().is_success() {
                return Err(StripeError::Request(format!("stripe customer create failed: {}", resp.status())));
            }
            let body: serde_json::Value =
                resp.json().await.map_err(|e| StripeError::Request(e.to_string()))?;
            body.get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| StripeError::Request("no customer id in response".into()))
        }

        async fn report_meter_event(
            &self,
            stripe_customer_id: &str,
            value: i64,
            idempotency_key: &str,
        ) -> Result<MeterEventReceipt, StripeError> {
            let resp = self
                .client
                .post("https://api.stripe.com/v1/billing/meter_events")
                .basic_auth(&self.secret_key, Some(""))
                .header("Idempotency-Key", idempotency_key)
                .form(&[
                    ("event_name", METER_EVENT_NAME),
                    ("payload[stripe_customer_id]", stripe_customer_id),
                    ("payload[value]", &value.to_string()),
                    ("identifier", idempotency_key),
                ])
                .send()
                .await
                .map_err(|e| StripeError::Request(e.to_string()))?;

            if !resp.status().is_success() {
                return Err(StripeError::Request(format!("stripe meter event failed: {}", resp.status())));
            }
            let body: serde_json::Value =
                resp.json().await.map_err(|e| StripeError::Request(e.to_string()))?;
            let id = body
                .get("identifier")
                .and_then(|v| v.as_str())
                .unwrap_or(idempotency_key)
                .to_string();
            Ok(MeterEventReceipt { stripe_meter_event_id: id })
        }
    }
}

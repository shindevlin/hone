//! Metering + billing glue: turns an aggregation query result into a
//! billable-or-not decision, writes the `usage_event` ledger row (+
//! `settlement_intent` rows) FIRST, then reports the Stripe Meter Event
//! with idempotency key = the usage_event id (PRD 1.4 section 2, verbatim
//! ordering requirement).

use std::sync::Arc;
use uuid::Uuid;

use crate::agg_client::{is_billable, GroupSnapshot, QueryParams, QueryResponse};
use crate::models::UsageEvent;
use crate::store::{BillableQuery, ProvenanceInput, Store, StoreError};
use crate::stripe_client::{StripeClient, StripeError};

#[derive(Debug, thiserror::Error)]
pub enum MeterError {
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    #[error("stripe error: {0}")]
    Stripe(#[from] StripeError),
}

/// Outcome of attempting to meter a query response.
pub enum MeterOutcome {
    /// Result was empty/failed — not billable, nothing written.
    NotBillable,
    /// Result was billable: ledger row written, Stripe reported (or the org
    /// has no stripe_customer_id yet, in which case reporting is skipped
    /// but the local ledger write still happened — nightly reconciliation
    /// will flag it as unreported until the org completes billing setup).
    Billed(UsageEvent),
}

/// Units-per-query default. A real "heavy query" multiplier (large geo x
/// long window, or export) would scale this per the PRD's published
/// units-table; v1 charges a flat 1 unit per billable query and the
/// multiplier is left as a documented follow-up (see report).
pub const DEFAULT_UNITS_PER_QUERY: i64 = 1;

/// Central metering entrypoint. Called by the query-builder route handler
/// after it gets a response back from the aggregation service.
pub async fn meter_query(
    store: &dyn Store,
    stripe: &dyn StripeClient,
    org_id: Uuid,
    stripe_customer_id: Option<&str>,
    api_key_id: Option<Uuid>,
    member_id: Option<Uuid>,
    params: &QueryParams,
    response: &QueryResponse,
) -> Result<MeterOutcome, MeterError> {
    if !is_billable(response) {
        return Ok(MeterOutcome::NotBillable);
    }

    let provenance = provenance_inputs(&response.groups);

    // usd_amount_cents_per_intent is a placeholder pending the real rate-plan
    // cents-per-unit lookup at the org's current plan; kept at 0 here since
    // the settlement bridge (separate item) resolves dreams_amount via its
    // own fx_rate_id, not from this dashboard. The dashboard's job is only
    // to ensure the intent row exists atomically with the usage_event.
    let input = BillableQuery {
        org_id,
        api_key_id,
        member_id,
        query_hash: params.stable_hash(),
        units: DEFAULT_UNITS_PER_QUERY,
        provenance,
        usd_amount_cents_per_intent: 0,
    };

    // Step 1 — ledger of record, written FIRST and unconditionally.
    let usage_event = store.record_billable_query(input).await?;

    // Step 2 — report to Stripe with idempotency key = usage_event.id, only
    // if the org has a Stripe customer yet (fresh orgs mid-signup may not).
    // If reporting fails or is skipped, the row stays in
    // list_unreported_usage() for nightly reconciliation to retry — the
    // local write already happened so a USD charge is never "lost", only
    // possibly delayed in reaching Stripe.
    if let Some(customer_id) = stripe_customer_id {
        let idempotency_key = usage_event.id.to_string();
        let receipt = stripe
            .report_meter_event(customer_id, usage_event.units, &idempotency_key)
            .await?;
        store
            .mark_usage_reported(usage_event.id, &receipt.stripe_meter_event_id)
            .await?;
    }

    Ok(MeterOutcome::Billed(usage_event))
}

fn provenance_inputs(groups: &[GroupSnapshot]) -> Vec<ProvenanceInput> {
    let mut out = Vec::new();
    for g in groups {
        for p in &g.provenance {
            out.push(ProvenanceInput {
                sensor_id: p.sensor_id.clone(),
                owner: p.owner.clone(),
                batch_hash: p.batch_hash.clone(),
            });
        }
    }
    out
}

/// Nightly reconciliation: sum local unreported usage_events per org and
/// (re-)report any that never made it to Stripe (crash between local write
/// and the Stripe call, or the org had no stripe_customer_id at query
/// time). Idempotency key = usage_event.id guarantees this is safe to run
/// repeatedly / concurrently with the live metering path.
pub async fn reconcile_unreported_usage(
    store: &dyn Store,
    stripe: &dyn StripeClient,
    org_id: Uuid,
    stripe_customer_id: &str,
) -> Result<usize, MeterError> {
    let unreported = store.list_unreported_usage(org_id).await?;
    let mut reported = 0usize;
    for event in unreported {
        let idempotency_key = event.id.to_string();
        let receipt = stripe.report_meter_event(stripe_customer_id, event.units, &idempotency_key).await?;
        store.mark_usage_reported(event.id, &receipt.stripe_meter_event_id).await?;
        reported += 1;
    }
    Ok(reported)
}

#[allow(dead_code)]
pub fn stripe_or_none(stripe: Arc<dyn StripeClient>) -> Arc<dyn StripeClient> {
    stripe
}

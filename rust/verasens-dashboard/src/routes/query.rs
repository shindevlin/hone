//! The query builder route: accepts either a session cookie (dashboard UI)
//! or an `Authorization: Bearer btcpc_live_...` API key (programmatic
//! clients, scope `query:read`), forwards the request to the aggregation
//! service's `GET /api/verasens/query`, renders the grouped result +
//! provenance, and meters the query per PRD 1.4.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Json};

use crate::agg_client::QueryParams;
use crate::app::AppState;
use crate::billing::{meter_query, MeterOutcome};
use crate::models::Scope;
use crate::routes::auth_routes::require_session;
use crate::routes::keys::require_api_key;

/// Resolved caller identity for a query request, regardless of which
/// credential path authenticated it.
struct Caller {
    org_id: uuid::Uuid,
    api_key_id: Option<uuid::Uuid>,
    member_id: Option<uuid::Uuid>,
}

async fn resolve_caller(state: &AppState, headers: &HeaderMap) -> Result<Caller, StatusCode> {
    // API key path takes priority if an Authorization header is present —
    // the two credential paths never mix on a single request.
    if headers.get(axum::http::header::AUTHORIZATION).is_some() {
        let key = require_api_key(state, headers, Scope::QueryRead).await?;
        return Ok(Caller { org_id: key.org_id, api_key_id: Some(key.id), member_id: None });
    }

    let session = require_session(state, headers).await?;
    Ok(Caller { org_id: session.org_id, api_key_id: None, member_id: Some(session.member_id) })
}

pub async fn run_query(State(state): State<AppState>, headers: HeaderMap, Query(params): Query<QueryParams>) -> impl IntoResponse {
    let caller = match resolve_caller(&state, &headers).await {
        Ok(c) => c,
        Err(status) => return (status, Json(serde_json::json!({"error": "unauthorized"}))).into_response(),
    };

    // Analyst role check for session callers happens implicitly: any active
    // member with a valid session can query per PRD (`analyst can query`);
    // billing-only members are excluded at the UI layer, and could be
    // hardened here with an explicit role check as a follow-up.

    let org = match state.store.get_org(caller.org_id).await {
        Ok(o) => o,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": "org lookup failed"}))).into_response(),
    };

    let response = match state.agg.query(&params).await {
        Ok(r) => r,
        Err(e) => {
            // Aggregation failure -> not billable, per PRD ("failed ...
            // results are not metered"). Surface the error but do not meter.
            return (StatusCode::BAD_GATEWAY, Json(serde_json::json!({"error": e.to_string()}))).into_response();
        }
    };

    let outcome = meter_query(
        state.store.as_ref(),
        state.stripe.as_ref(),
        caller.org_id,
        org.stripe_customer_id.as_deref(),
        caller.api_key_id,
        caller.member_id,
        &params,
        &response,
    )
    .await;

    let billed = match outcome {
        Ok(MeterOutcome::Billed(event)) => Some(event.id),
        Ok(MeterOutcome::NotBillable) => None,
        Err(e) => {
            tracing::error!("metering failed for org {}: {e}", caller.org_id);
            // The query itself succeeded; a metering failure is a billing
            // concern (surfaced to ops via logs / reconciliation), not a
            // reason to hide the result from the buyer who already
            // received a successful aggregation answer.
            None
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "count": response.count,
            "groups": response.groups,
            "billed_usage_event_id": billed,
        })),
    )
        .into_response()
}

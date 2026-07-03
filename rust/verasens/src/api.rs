//! Axum HTTP query + ingest API.
//!
//! Routes (all under `/api/verasens`, matching the `btcpc-market`/`linkgit`
//! convention of a `/api/<service>` namespace):
//!
//! - `GET  /health` — liveness + index counters.
//! - `POST /ingest` — accept a batch of raw sensor readings (JSON), parse
//!   and fold them into the aggregation index. This is the thin adapter
//!   surface a live-chain poller (or a manual backfill script) posts to.
//! - `GET  /query` — the actual product: aggregated groups matching a
//!   class/geo/time filter. Returns grouped stats + verified/unverified
//!   breakdown + provenance, never raw per-reading dumps.
//! - `GET  /groups/:class/:geo/:window_epochs/:window_index` — exact lookup
//!   of a single group by its full key.

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use crate::agg::QueryFilter;
use crate::ingest::{raw_reading_to_entry, IngestItem, RawSensorReading};
use crate::model::GeoBucket;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ingest", post(ingest))
        .route("/query", get(query))
        .route(
            "/groups/:class/:geo/:window_epochs/:window_index",
            get(get_group),
        )
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    let stats = state.index.read().stats();
    Json(serde_json::json!({
        "ok": true,
        "service": "verasens",
        "version": env!("CARGO_PKG_VERSION"),
        "index": stats,
    }))
}

/// Ingest request body: a batch of raw readings plus how much to trust them.
/// `source` is per-batch for simplicity (a single POST is expected to come
/// from one adapter run with one provenance story); mixed-trust batches can
/// just be split into separate POSTs by the caller.
#[derive(Debug, Deserialize)]
struct IngestRequest {
    readings: Vec<RawSensorReading>,
    /// "chain_confirmed" (trust each reading's own `signed_by`) or
    /// "unconfirmed" (always mark unverified). Defaults to "unconfirmed" —
    /// callers must opt in to trusting signed_by, matching the fail-closed
    /// posture the audit calls for.
    #[serde(default = "default_source")]
    source: String,
}

fn default_source() -> String {
    "unconfirmed".to_string()
}

async fn ingest(
    State(state): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> impl IntoResponse {
    let items: Vec<IngestItem> = req
        .readings
        .into_iter()
        .map(|raw| {
            let entry = raw_reading_to_entry(raw);
            if req.source == "chain_confirmed" {
                IngestItem::chain_confirmed(entry)
            } else {
                IngestItem::unconfirmed(entry)
            }
        })
        .collect();

    let (newly, dupes, rejected) = state.ingest_items(&items);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ingested": newly,
            "duplicates_skipped": dupes,
            "rejected": rejected,
        })),
    )
}

#[derive(Debug, Deserialize)]
struct QueryParams {
    class: Option<String>,
    /// Geo bucket key, `"lat_q:lon_q"` (see `GeoBucket::key`).
    geo: Option<String>,
    window_index: Option<u64>,
    epoch_from: Option<u64>,
    epoch_to: Option<u64>,
}

async fn query(State(state): State<AppState>, Query(params): Query<QueryParams>) -> impl IntoResponse {
    let geo = match params.geo.as_deref().map(GeoBucket::parse_key) {
        Some(Some(g)) => Some(g),
        Some(None) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error": "invalid geo bucket key, expected 'lat_q:lon_q'"})),
            )
                .into_response()
        }
        None => None,
    };

    let filter = QueryFilter {
        class: params.class,
        geo,
        window_index: params.window_index,
        epoch_from: params.epoch_from,
        epoch_to: params.epoch_to,
    };

    let groups = state.index.read().query(&filter);
    // Attach client-buildable lookup keys (geo_key/window_key) alongside each
    // group so a caller can round-trip a result straight into
    // GET /groups/:class/:geo/:window_epochs/:window_index without having to
    // re-derive the grid/window encoding itself.
    let annotated: Vec<serde_json::Value> = groups
        .iter()
        .map(|g| {
            let mut v = serde_json::to_value(g).unwrap_or_default();
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "geo_key".to_string(),
                    g.key
                        .geo
                        .as_ref()
                        .map(|geo| serde_json::json!(geo.key()))
                        .unwrap_or(serde_json::json!(null)),
                );
                obj.insert("window_key".to_string(), serde_json::json!(g.key.window.key()));
            }
            v
        })
        .collect();

    Json(serde_json::json!({
        "count": annotated.len(),
        "groups": annotated,
    }))
    .into_response()
}

async fn get_group(
    State(state): State<AppState>,
    Path((class, geo, window_epochs, window_index)): Path<(String, String, u64, u64)>,
) -> impl IntoResponse {
    let geo_bucket = if geo == "none" {
        None
    } else {
        match GeoBucket::parse_key(&geo) {
            Some(g) => Some(g),
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({"error": "invalid geo bucket key"})),
                )
                    .into_response()
            }
        }
    };

    let key = crate::model::GroupKey {
        class,
        geo: geo_bucket,
        window: crate::model::TimeWindow {
            index: window_index,
            window_epochs,
        },
    };

    match state.index.read().get(&key) {
        Some(snapshot) => Json(snapshot).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "group not found"})))
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt; // for `oneshot`

    fn sample_reading_json() -> serde_json::Value {
        serde_json::json!({
            "sensor_id": "sensor-1",
            "owner": "alice",
            "epoch": 10,
            "value": 5.0,
            "data_hash": "h1",
            "metadata": {
                "class": "gnss",
                "schema": "1",
                "lat": 53.349,
                "lon": -6.260,
                "payload": {"alt_m": 10.0}
            },
            "signed_by": "alice"
        })
    }

    #[tokio::test]
    async fn health_returns_ok() {
        let app = router(AppState::default_window());
        let resp = app
            .oneshot(Request::builder().uri("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn ingest_then_query_round_trip() {
        let app = router(AppState::default_window());

        let body = serde_json::json!({
            "readings": [sample_reading_json()],
            "source": "chain_confirmed"
        });

        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ingest")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/query?class=gnss")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["count"], 1);
        assert_eq!(json["groups"][0]["count"], 1);
        assert_eq!(json["groups"][0]["verified_count"], 1);
    }

    #[tokio::test]
    async fn unconfirmed_source_marks_readings_unverified() {
        let app = router(AppState::default_window());
        let body = serde_json::json!({
            "readings": [sample_reading_json()],
            "source": "unconfirmed"
        });
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ingest")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        let resp = app
            .oneshot(Request::builder().uri("/query?class=gnss").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["groups"][0]["verified_count"], 0);
        assert_eq!(json["groups"][0]["unverified_count"], 1);
    }
}

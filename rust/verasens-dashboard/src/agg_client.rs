//! Client for the Phase 1.2 Verasens aggregation service's public query API.
//!
//! Per the PRD, the dashboard talks to the aggregation service ONLY through
//! this HTTP contract (`GET /api/verasens/query`) — never chain state,
//! never aggregation internals. Route/field names here mirror
//! `rust/verasens/src/api.rs` and `agg.rs` on branch
//! `platform/p1-verasens-aggregation` exactly.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryParams {
    pub class: Option<String>,
    pub geo: Option<String>,
    pub window_index: Option<u64>,
    pub epoch_from: Option<u64>,
    pub epoch_to: Option<u64>,
}

impl QueryParams {
    /// Stable hash of the query shape, used both as `usage_event.query_hash`
    /// and as part of dedup/audit trails. Order-independent w.r.t. field
    /// presence since we hash a canonical fixed-field string.
    pub fn stable_hash(&self) -> String {
        let canonical = format!(
            "class={}&geo={}&window_index={}&epoch_from={}&epoch_to={}",
            self.class.as_deref().unwrap_or(""),
            self.geo.as_deref().unwrap_or(""),
            self.window_index.map(|v| v.to_string()).unwrap_or_default(),
            self.epoch_from.map(|v| v.to_string()).unwrap_or_default(),
            self.epoch_to.map(|v| v.to_string()).unwrap_or_default(),
        );
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        hex::encode(hasher.finalize())
    }

    pub fn to_query_string(&self) -> String {
        let mut parts = Vec::new();
        if let Some(c) = &self.class {
            parts.push(format!("class={}", urlencoding_lite(c)));
        }
        if let Some(g) = &self.geo {
            parts.push(format!("geo={}", urlencoding_lite(g)));
        }
        if let Some(w) = self.window_index {
            parts.push(format!("window_index={w}"));
        }
        if let Some(e) = self.epoch_from {
            parts.push(format!("epoch_from={e}"));
        }
        if let Some(e) = self.epoch_to {
            parts.push(format!("epoch_to={e}"));
        }
        parts.join("&")
    }
}

fn urlencoding_lite(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Provenance entry as returned by the aggregation service's `/query`
/// response, per `rust/verasens/src/agg.rs::Provenance`. NOTE (per task
/// scope): `batch_hash` is NOT yet present in the live response — that gap
/// is owned by the sibling settlement-bridge item, not this dashboard. We
/// model the field as optional so the dashboard's settlement_intent writes
/// degrade gracefully (`batch_hash: None`) until that gap closes, rather
/// than breaking on the current contract shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub sensor_id: String,
    pub owner: String,
    pub data_hash: String,
    pub epoch: u64,
    pub verified: bool,
    #[serde(default)]
    pub batch_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueStats {
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    pub p50: f64,
    pub p95: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupSnapshot {
    pub key: serde_json::Value,
    pub count: u64,
    pub verified_count: u64,
    pub unverified_count: u64,
    pub stats: Option<ValueStats>,
    pub provenance: Vec<Provenance>,
    #[serde(default)]
    pub geo_key: Option<String>,
    #[serde(default)]
    pub window_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResponse {
    pub count: usize,
    pub groups: Vec<GroupSnapshot>,
}

#[derive(Debug, thiserror::Error)]
pub enum AggError {
    #[error("aggregation request failed: {0}")]
    Request(String),
    #[error("aggregation returned error status {0}")]
    Status(u16),
}

#[async_trait]
pub trait AggregationClient: Send + Sync {
    async fn query(&self, params: &QueryParams) -> Result<QueryResponse, AggError>;
}

pub struct HttpAggregationClient {
    base_url: String,
    client: reqwest::Client,
}

impl HttpAggregationClient {
    pub fn new(base_url: String) -> Self {
        Self { base_url, client: reqwest::Client::new() }
    }
}

#[async_trait]
impl AggregationClient for HttpAggregationClient {
    async fn query(&self, params: &QueryParams) -> Result<QueryResponse, AggError> {
        let url = format!("{}/api/verasens/query?{}", self.base_url.trim_end_matches('/'), params.to_query_string());
        let resp = self.client.get(&url).send().await.map_err(|e| AggError::Request(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(AggError::Status(resp.status().as_u16()));
        }
        resp.json::<QueryResponse>().await.map_err(|e| AggError::Request(e.to_string()))
    }
}

/// Whether an aggregation response counts as a "billable query" per PRD 1.4
/// section 2: "successful, non-empty result; failed/empty results are not
/// metered."
pub fn is_billable(resp: &QueryResponse) -> bool {
    resp.count > 0 && !resp.groups.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_hash_is_deterministic_and_field_sensitive() {
        let a = QueryParams { class: Some("gnss".into()), ..Default::default() };
        let b = QueryParams { class: Some("gnss".into()), ..Default::default() };
        let c = QueryParams { class: Some("ble".into()), ..Default::default() };
        assert_eq!(a.stable_hash(), b.stable_hash());
        assert_ne!(a.stable_hash(), c.stable_hash());
    }

    #[test]
    fn empty_result_is_not_billable() {
        let resp = QueryResponse { count: 0, groups: vec![] };
        assert!(!is_billable(&resp));
    }

    #[test]
    fn nonempty_result_is_billable() {
        let resp = QueryResponse {
            count: 1,
            groups: vec![GroupSnapshot {
                key: serde_json::json!({}),
                count: 3,
                verified_count: 3,
                unverified_count: 0,
                stats: None,
                provenance: vec![],
                geo_key: None,
                window_key: None,
            }],
        };
        assert!(is_billable(&resp));
    }
}

//! HTTP client for the local hone-node API.
//!
//! Skeleton only: read-only capability/worker discovery today. Job-posting routes
//! land as the chain-side `Wiiv*` entries are implemented; until then the client
//! exposes health/discovery and the MCP layer drives dry-run planning.

use anyhow::{Context, Result};
use serde::Deserialize;

/// A thin client bound to a node API base and (optionally) a signing identity.
pub struct Client {
    base: String,
    account: String,
    posting_key: String,
    http: reqwest::Client,
}

/// A worker's advertised capability listing, as returned by discovery.
#[derive(Debug, Clone, Deserialize)]
pub struct WorkerListing {
    pub account: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub modalities: Vec<String>,
    #[serde(default)]
    pub price_hint_hunits: Option<u64>,
}

impl Client {
    pub fn new(base: String, account: String, posting_key: String) -> Self {
        Self {
            base,
            account,
            posting_key,
            http: reqwest::Client::new(),
        }
    }

    /// Resolve the API base from HONE_* env, matching the other HONE clients.
    pub fn from_env() -> Self {
        let base = std::env::var("HONE_API").unwrap_or_else(|_| "http://localhost:4242".to_string());
        let account = std::env::var("HONE_ACCOUNT").unwrap_or_default();
        let posting_key = std::env::var("HONE_POSTING_KEY").unwrap_or_default();
        Self::new(base, account, posting_key)
    }

    pub fn account(&self) -> &str {
        &self.account
    }

    pub fn has_signing_identity(&self) -> bool {
        !self.account.is_empty() && !self.posting_key.is_empty()
    }

    /// Node liveness / info probe.
    pub async fn node_info(&self) -> Result<serde_json::Value> {
        let url = format!("{}/api/node/info", self.base);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        Ok(resp.json().await?)
    }

    /// Discover live Wiiv workers matching the given capability, once the node
    /// exposes the registry route. Returns an empty list until then.
    pub async fn discover_workers(&self, _capability: &str) -> Result<Vec<WorkerListing>> {
        // Placeholder: the `/api/wiiv/workers` route is not implemented yet.
        // Kept as a typed seam so callers can be written against it now.
        Ok(Vec::new())
    }
}

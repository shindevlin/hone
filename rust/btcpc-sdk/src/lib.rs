//! BTCPC chain client SDK.
//!
//! # Quick start
//!
//! ```no_run
//! use btcpc_sdk::BtcpcClient;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let client = BtcpcClient::new("http://localhost:4242");
//!     let latest = client.latest().await?;
//!     println!("Current epoch: {}", latest.epoch);
//!     Ok(())
//! }
//! ```

use anyhow::Result;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::{Deserialize, Serialize};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Number of dreams in one BTCPC (10^10).
pub const DREAMS_PER_BTCPC: u64 = 10_000_000_000;

/// Native token ticker.
pub const NATIVE_TOKEN: &str = "BTCPC";

/// Default local node API address.
pub const DEFAULT_API_URL: &str = "http://localhost:4242";

// ── Response types ────────────────────────────────────────────────────────────

/// Response from `GET /api/balance/:account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceResponse {
    pub account: String,
    /// Balance in BTCPC (fractional).
    pub balance: f64,
    /// Balance in dreams (smallest unit).
    pub dreams: u64,
    pub token: String,
}

/// Response from `GET /api/stake/:account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakeResponse {
    pub account: String,
    /// Staked amount in BTCPC.
    pub stake: f64,
    /// Staked amount in dreams.
    pub dreams: u64,
}

/// Response from `GET /api/latest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestResponse {
    /// Latest finalized epoch number.
    pub epoch: u32,
    /// Block hash hex of the latest epoch.
    pub hash: String,
    /// The node's internal current epoch counter.
    pub current_epoch: u64,
}

/// Response from transaction submission endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxResponse {
    /// Transaction hash, present when `accepted` is `true`.
    pub hash: Option<String>,
    pub accepted: bool,
    pub error: Option<String>,
}

/// Response from `POST /api/contract/deploy`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployResponse {
    pub contract_id: Option<String>,
    pub ok: bool,
    pub error: Option<String>,
}

// ── Client ────────────────────────────────────────────────────────────────────

/// HTTP client for the BTCPC node API.
///
/// Build with [`BtcpcClient::new`], optionally attach a default account with
/// [`BtcpcClient::with_account`], or load from environment with
/// [`BtcpcClient::from_env`].
pub struct BtcpcClient {
    base_url: String,
    account: Option<String>,
    http: reqwest::Client,
}

impl BtcpcClient {
    /// Create a client pointing at `base_url` (e.g. `"http://localhost:4242"`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            account: None,
            http: reqwest::Client::new(),
        }
    }

    /// Attach a default account name to the client (used as a convenience
    /// handle; individual methods still accept explicit account arguments).
    pub fn with_account(mut self, account: impl Into<String>) -> Self {
        self.account = Some(account.into());
        self
    }

    /// Build a client from environment variables:
    /// - `BTCPC_API_URL`  — defaults to [`DEFAULT_API_URL`]
    /// - `BTCPC_ACCOUNT`  — optional default account
    pub fn from_env() -> Self {
        let base_url = std::env::var("BTCPC_API_URL")
            .unwrap_or_else(|_| DEFAULT_API_URL.to_owned());
        let account = std::env::var("BTCPC_ACCOUNT").ok();
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            account,
            http: reqwest::Client::new(),
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    // ── Chain queries ─────────────────────────────────────────────────────────

    /// Fetch the BTCPC balance of `account`.
    pub async fn balance(&self, account: &str) -> Result<BalanceResponse> {
        let resp = self
            .http
            .get(self.url(&format!("/api/balance/{}", account)))
            .send()
            .await?
            .error_for_status()?
            .json::<BalanceResponse>()
            .await?;
        Ok(resp)
    }

    /// Fetch the staked BTCPC of `account`.
    pub async fn stake(&self, account: &str) -> Result<StakeResponse> {
        let resp = self
            .http
            .get(self.url(&format!("/api/stake/{}", account)))
            .send()
            .await?
            .error_for_status()?
            .json::<StakeResponse>()
            .await?;
        Ok(resp)
    }

    /// Fetch the latest finalized block info.
    pub async fn latest(&self) -> Result<LatestResponse> {
        let resp = self
            .http
            .get(self.url("/api/latest"))
            .send()
            .await?
            .error_for_status()?
            .json::<LatestResponse>()
            .await?;
        Ok(resp)
    }

    /// Fetch a block by epoch number, returning raw JSON.
    pub async fn block(&self, epoch: u32) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(self.url(&format!("/api/block/{}", epoch)))
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp)
    }

    /// Ping the node health endpoint. Returns `true` if the node responds with
    /// `status: "ok"`.
    pub async fn health(&self) -> Result<bool> {
        let resp = self
            .http
            .get(self.url("/health"))
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp.get("status").and_then(|v| v.as_str()) == Some("ok"))
    }

    // ── Transactions ──────────────────────────────────────────────────────────

    /// Submit a BTCPC transfer.
    ///
    /// Pass an empty string for `sig` to submit unsigned (accepted by
    /// permissive nodes or for testing).
    pub async fn transfer(
        &self,
        from: &str,
        to: &str,
        amount_btcpc: f64,
        memo: Option<&str>,
        nonce: u64,
        sig: &str,
    ) -> Result<TxResponse> {
        let body = serde_json::json!({
            "from": from,
            "to": to,
            "amount": amount_btcpc,
            "memo": memo,
            "signed_by": from,
            "nonce": nonce,
            "signature": sig,
        });
        self.post_tx("/api/transfer", body).await
    }

    /// Submit a stake addition.
    pub async fn stake_add(&self, account: &str, amount_btcpc: f64, sig: &str) -> Result<TxResponse> {
        let body = serde_json::json!({
            "account": account,
            "amount": amount_btcpc,
            "signed_by": account,
            "signature": sig,
        });
        self.post_tx("/api/stake", body).await
    }

    /// Submit a stake removal (unstake).
    pub async fn stake_remove(&self, account: &str, amount_btcpc: f64, sig: &str) -> Result<TxResponse> {
        let body = serde_json::json!({
            "account": account,
            "amount": amount_btcpc,
            "signed_by": account,
            "signature": sig,
        });
        self.post_tx("/api/unstake", body).await
    }

    /// Create a new account on-chain.
    ///
    /// `public_key` should be a hex-encoded ed25519 public key, or `None` for
    /// a watch-only account.
    pub async fn account_create(
        &self,
        account: &str,
        public_key: Option<&str>,
    ) -> Result<TxResponse> {
        let body = serde_json::json!({
            "account": account,
            "public_key": public_key,
        });
        self.post_tx("/api/account/create", body).await
    }

    // ── Contracts ─────────────────────────────────────────────────────────────

    /// Deploy a WASM contract.
    ///
    /// `wasm_bytes` will be base64-encoded before posting.
    pub async fn contract_deploy(
        &self,
        deployer: &str,
        wasm_bytes: &[u8],
        init_method: Option<&str>,
        init_args: Option<serde_json::Value>,
        gas: u64,
    ) -> Result<DeployResponse> {
        let wasm_b64 = B64.encode(wasm_bytes);
        let body = serde_json::json!({
            "deployer": deployer,
            "wasm_b64": wasm_b64,
            "init_method": init_method,
            "init_args": init_args,
            "gas": gas,
        });
        let resp = self
            .http
            .post(self.url("/api/contract/deploy"))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<DeployResponse>()
            .await?;
        Ok(resp)
    }

    /// Call a contract method (state-mutating).
    ///
    /// Returns the contract's return value as raw JSON.
    pub async fn contract_call(
        &self,
        contract_id: &str,
        method: &str,
        args: serde_json::Value,
        signer: &str,
        deposit_btcpc: f64,
        gas: u64,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "contract_id": contract_id,
            "method": method,
            "args": args,
            "signer": signer,
            "deposit": deposit_btcpc,
            "gas": gas,
        });
        let resp = self
            .http
            .post(self.url("/api/contract/call"))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp)
    }

    /// Call a contract view method (read-only, no state change).
    ///
    /// Returns the contract's return value as raw JSON.
    pub async fn contract_view(
        &self,
        contract_id: &str,
        method: &str,
        args: serde_json::Value,
        gas: u64,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "contract_id": contract_id,
            "method": method,
            "args": args,
            "gas": gas,
        });
        let resp = self
            .http
            .post(self.url("/api/contract/view"))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp)
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    async fn post_tx(&self, path: &str, body: serde_json::Value) -> Result<TxResponse> {
        let resp = self
            .http
            .post(self.url(path))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<TxResponse>()
            .await?;
        Ok(resp)
    }
}

// ── Utility helpers ───────────────────────────────────────────────────────────

/// Convert BTCPC (fractional) to dreams (integer, smallest unit).
///
/// ```
/// assert_eq!(btcpc_sdk::btcpc_to_dreams(1.0), 10_000_000_000);
/// assert_eq!(btcpc_sdk::btcpc_to_dreams(0.5), 5_000_000_000);
/// ```
pub fn btcpc_to_dreams(btcpc: f64) -> u64 {
    (btcpc * DREAMS_PER_BTCPC as f64).round() as u64
}

/// Convert dreams (integer) to BTCPC (fractional).
///
/// ```
/// assert_eq!(btcpc_sdk::dreams_to_btcpc(10_000_000_000), 1.0);
/// ```
pub fn dreams_to_btcpc(dreams: u64) -> f64 {
    dreams as f64 / DREAMS_PER_BTCPC as f64
}

//! HONE chain client SDK.
//!
//! # Quick start
//!
//! ```no_run
//! use hone_sdk::HoneClient;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let client = HoneClient::new("http://localhost:4242");
//!     let latest = client.latest().await?;
//!     println!("Current epoch: {}", latest.epoch);
//!     Ok(())
//! }
//! ```

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

/// The self-updating integration manifest: how any repo in the HONE ecosystem
/// keeps a current, diffable understanding of the chain's surface. See
/// [`manifest`] for the full design.
pub mod manifest;

/// Recoverable encrypted keystore (Argon2id + AES-256-GCM). Every wallet created
/// after Genesis v2 gets one, so a mnemonic can never be silently lost. See
/// [`keystore`] and `docs/GENESIS_JULY4_2026.md`.
pub mod keystore;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Number of hunits in one HONE (10^10).
pub const HUNITS_PER_HONE: u64 = 10_000_000_000;

/// Native token ticker.
pub const NATIVE_TOKEN: &str = "HONE";

/// Default local node API address.
pub const DEFAULT_API_URL: &str = "http://localhost:4242";

// ── Response types ────────────────────────────────────────────────────────────

/// Response from `GET /api/balance/:account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceResponse {
    pub account: String,
    /// Balance in hunits (smallest unit).
    pub hunits: u64,
    pub token: String,
}

/// Response from `GET /api/stake/:account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakeResponse {
    pub account: String,
    /// Staked amount in hunits.
    pub hunits: u64,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AllBalancesResponse {
    balances: HashMap<String, serde_json::Value>,
}

// ── Client ────────────────────────────────────────────────────────────────────

/// HTTP client for the HONE node API.
///
/// Build with [`HoneClient::new`], optionally attach a default account with
/// [`HoneClient::with_account`], or load from environment with
/// [`HoneClient::from_env`].
pub struct HoneClient {
    base_url: String,
    account: Option<String>,
    /// Bearer token sent in `Authorization: Bearer <api_key>` for paid endpoints.
    /// Currently equals your account name; rotate-able tokens land in a future release.
    api_key: Option<String>,
    http: reqwest::Client,
}

impl HoneClient {
    /// Create a client pointing at `base_url` (e.g. `"http://localhost:4242"`).
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            account: None,
            api_key: None,
            http: reqwest::Client::new(),
        }
    }

    /// Attach a default account name to the client (used as a convenience
    /// handle; individual methods still accept explicit account arguments).
    pub fn with_account(mut self, account: impl Into<String>) -> Self {
        self.account = Some(account.into());
        self
    }

    /// Attach an API key (Bearer token for paid endpoints such as `/v1/chat/completions`).
    /// Currently the API key is your account name. Set via `HONE_API_KEY` in env.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Build a client from environment variables:
    /// - `HONE_API_URL`   — defaults to [`DEFAULT_API_URL`]
    /// - `HONE_ACCOUNT`   — optional default account name
    /// - `HONE_API_KEY`   — API key for paid endpoints (currently equals account name)
    pub fn from_env() -> Self {
        let base_url = std::env::var("HONE_API_URL")
            .unwrap_or_else(|_| DEFAULT_API_URL.to_owned());
        let account = std::env::var("HONE_ACCOUNT").ok();
        // HONE_API_KEY is the Bearer token for paid calls. Falls back to HONE_ACCOUNT
        // so callers that only set one variable still work.
        let api_key = std::env::var("HONE_API_KEY").ok()
            .or_else(|| account.clone());
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            account,
            api_key,
            http: reqwest::Client::new(),
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    // ── Chain queries ─────────────────────────────────────────────────────────

    /// Fetch the HONE balance of `account`.
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

    /// Fetch the staked HONE of `account`.
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

    // ── Inference ─────────────────────────────────────────────────────────────

    /// Send an OpenAI-compatible chat completion request to the HONE inference
    /// gateway at `/v1/chat/completions`.
    ///
    /// Requires `HONE_API_KEY` (or `HONE_ACCOUNT`) to be set — the fee of
    /// 10 000 hunits (0.0001 HONE) is debited per call.
    ///
    /// `model` defaults to the node's active model when `None`.
    pub async fn chat_completions(
        &self,
        messages: Vec<serde_json::Value>,
        model: Option<&str>,
    ) -> Result<serde_json::Value> {
        let key = self.api_key.as_deref()
            .ok_or_else(|| anyhow!("HONE_API_KEY not set — cannot call paid inference endpoint"))?;

        let mut body = serde_json::json!({ "messages": messages });
        if let Some(m) = model {
            body["model"] = serde_json::Value::String(m.to_owned());
        }

        let resp = self
            .http
            .post(self.url("/v1/chat/completions"))
            .header("Authorization", format!("Bearer {}", key))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp)
    }

    // ── Faucet ────────────────────────────────────────────────────────────────

    /// Claim testnet tokens from the faucet for `account`.
    /// Returns the raw JSON response (accepted / error).
    pub async fn faucet_claim(&self, account: &str) -> Result<serde_json::Value> {
        let resp = self
            .http
            .post(self.url("/api/faucet/claim"))
            .json(&serde_json::json!({ "account": account }))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp)
    }

    /// Fetch all token balances of `account` as hunits (u64).
    pub async fn all_balances(&self, account: &str) -> Result<HashMap<String, u64>> {
        let resp = self
            .http
            .get(self.url(&format!("/api/balances/{}", account)))
            .send()
            .await?
            .error_for_status()?
            .json::<AllBalancesResponse>()
            .await?;

        let mut out = HashMap::new();
        for (token, value) in resp.balances {
            out.insert(token, parse_hunits_value(&value)?);
        }
        Ok(out)
    }

    /// Fetch account metadata by account name.
    pub async fn account(&self, account: &str) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(self.url(&format!("/api/account/{}", account)))
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp)
    }

    /// Fetch epoch metadata by epoch number.
    pub async fn epoch_meta(&self, epoch: u64) -> Result<serde_json::Value> {
        let resp = self
            .http
            .get(self.url(&format!("/api/epoch/{}", epoch)))
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        Ok(resp)
    }

    /// Fetch the current account nonce from `/api/account/:account`.
    pub async fn get_nonce(&self, account: &str) -> Result<u64> {
        let account_data = self.account(account).await?;
        let nonce_value = account_data
            .get("nonce")
            .ok_or_else(|| anyhow!("account '{}' has no nonce field", account))?;
        parse_hunits_value(nonce_value)
    }

    /// Return the next valid nonce (`current + 1`) for an account.
    pub async fn next_nonce(&self, account: &str) -> Result<u64> {
        let current = self.get_nonce(account).await?;
        current
            .checked_add(1)
            .ok_or_else(|| anyhow!("nonce overflow for account '{}'", account))
    }

    // ── Transactions ──────────────────────────────────────────────────────────

    /// Submit a HONE transfer.
    ///
    /// Pass an empty string for `sig` to submit unsigned (accepted by
    /// permissive nodes or for testing).
    pub async fn transfer(
        &self,
        from: &str,
        to: &str,
        amount_hunits: u64,
        memo: Option<&str>,
        nonce: u64,
        sig: &str,
    ) -> Result<TxResponse> {
        let body = serde_json::json!({
            "from": from,
            "to": to,
            "amount": amount_hunits,
            "memo": memo,
            "token": NATIVE_TOKEN,
            "signed_by": from,
            "nonce": nonce,
            "signature": sig,
        });
        self.post_tx("/api/transfer", body).await
    }

    /// Submit a stake addition.
    pub async fn stake_add(
        &self,
        account: &str,
        amount_hunits: u64,
        nonce: u64,
        sig: &str,
    ) -> Result<TxResponse> {
        let body = serde_json::json!({
            "account": account,
            "amount": amount_hunits,
            "signed_by": account,
            "nonce": nonce,
            "signature": sig,
        });
        self.post_tx("/api/stake", body).await
    }

    /// Submit a stake removal (unstake).
    pub async fn stake_remove(
        &self,
        account: &str,
        amount_hunits: u64,
        nonce: u64,
        sig: &str,
    ) -> Result<TxResponse> {
        let body = serde_json::json!({
            "account": account,
            "amount": amount_hunits,
            "signed_by": account,
            "nonce": nonce,
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
        nonce: u64,
        signature: &str,
    ) -> Result<DeployResponse> {
        let wasm_b64 = B64.encode(wasm_bytes);
        let body = serde_json::json!({
            "deployer": deployer,
            "wasm_b64": wasm_b64,
            "init_method": init_method,
            "init_args": init_args,
            "gas": gas,
            "nonce": nonce,
            "signature": signature,
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
        deposit_hunits: u64,
        gas: u64,
        nonce: u64,
        signature: &str,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "contract_id": contract_id,
            "method": method,
            "args": args,
            "signer": signer,
            "deposit": deposit_hunits,
            "gas": gas,
            "nonce": nonce,
            "signature": signature,
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

    // ── TOTP 2FA (P1-B) ───────────────────────────────────────────────────────

    pub async fn totp_setup(&self, account: &str, nonce: u64, sig: &str) -> Result<serde_json::Value> {
        self.http.post(self.url("/api/totp/setup"))
            .json(&serde_json::json!({ "account": account, "nonce": nonce, "signature": sig }))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn totp_enable(&self, account: &str, code: &str, nonce: u64, sig: &str) -> Result<TxResponse> {
        self.post_tx("/api/totp/enable",
            serde_json::json!({ "account": account, "code": code, "nonce": nonce, "signature": sig })).await
    }

    pub async fn totp_verify(&self, account: &str, code: &str) -> Result<serde_json::Value> {
        self.http.post(self.url("/api/totp/verify"))
            .json(&serde_json::json!({ "account": account, "code": code }))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn totp_disable(&self, account: &str, code: &str, nonce: u64, sig: &str) -> Result<TxResponse> {
        self.post_tx("/api/totp/disable",
            serde_json::json!({ "account": account, "code": code, "nonce": nonce, "signature": sig })).await
    }

    pub async fn totp_backup_codes(&self, account: &str, nonce: u64, sig: &str) -> Result<serde_json::Value> {
        self.http.post(self.url("/api/totp/backup-codes"))
            .json(&serde_json::json!({ "account": account, "nonce": nonce, "signature": sig }))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Streaming Inference SSE (P1-C) ────────────────────────────────────────

    /// Returns the raw `reqwest::Response` for the caller to consume as an SSE stream.
    pub async fn inference_stream(
        &self,
        account: &str,
        prompt: &str,
        model: Option<&str>,
    ) -> Result<reqwest::Response> {
        let key = self.api_key.as_deref()
            .ok_or_else(|| anyhow!("HONE_API_KEY required for paid inference endpoint"))?;
        let mut body = serde_json::json!({ "account": account, "prompt": prompt });
        if let Some(m) = model { body["model"] = serde_json::Value::String(m.to_owned()); }
        let resp = self.http.post(self.url("/api/inference/stream"))
            .header("Authorization", format!("Bearer {}", key))
            .json(&body).send().await?.error_for_status()?;
        Ok(resp)
    }

    /// Stream a settled inference job (replayed as SSE).
    pub async fn task_job_stream(&self, job_id: &str) -> Result<reqwest::Response> {
        self.http.get(self.url(&format!("/api/task/job/{}/stream", job_id)))
            .send().await?.error_for_status().map_err(Into::into)
    }

    // ── Phone Mining (P1-E) ───────────────────────────────────────────────────

    pub async fn phone_mine_claim(
        &self, account: &str, device_id: &str, nonce: u64, sig: &str,
    ) -> Result<serde_json::Value> {
        self.http.post(self.url("/api/mining/phone/claim"))
            .json(&serde_json::json!({ "account": account, "device_id": device_id, "nonce": nonce, "signature": sig }))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn phone_mine_submit(
        &self, account: &str, job_id: &str, work_hash: &str,
        device_id: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/mining/phone/submit", serde_json::json!({
            "account": account, "job_id": job_id, "work_hash": work_hash,
            "device_id": device_id, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn phone_mine_status(&self, account: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/mining/phone/status?account={}", account)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Mempool Fee Market (P1-F) ─────────────────────────────────────────────

    pub async fn fee_estimate(&self) -> Result<serde_json::Value> {
        self.http.get(self.url("/api/chain/fee-estimate"))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn mempool_status(&self) -> Result<serde_json::Value> {
        self.http.get(self.url("/api/mempool/status"))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Ensemble Coordinator (P2-B) ───────────────────────────────────────────

    pub async fn ensemble_job_post(
        &self, account: &str, prompt: &str, model: &str,
        n_workers: u32, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/ensemble/job", serde_json::json!({
            "account": account, "prompt": prompt, "model": model,
            "n_workers": n_workers, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn ensemble_vote(
        &self, account: &str, job_id: &str, output: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx(&format!("/api/ensemble/vote/{}", job_id), serde_json::json!({
            "account": account, "output": output, "nonce": nonce, "signature": sig,
        })).await
    }

    // ── Slashing Framework (P2-D) ─────────────────────────────────────────────

    pub async fn slash_validator(
        &self, reporter: &str, target: &str, evidence: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/slash", serde_json::json!({
            "reporter": reporter, "target": target,
            "evidence": evidence, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn slash_appeal(
        &self, account: &str, slash_id: &str, appeal_text: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/slash/appeal", serde_json::json!({
            "account": account, "slash_id": slash_id,
            "appeal_text": appeal_text, "nonce": nonce, "signature": sig,
        })).await
    }

    // ── Bridge Registry (P2-E) ────────────────────────────────────────────────

    pub async fn bridge_wrap(
        &self, account: &str, amount_hunits: u64, chain: &str,
        dest_address: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/bridge/wrap", serde_json::json!({
            "account": account, "amount": amount_hunits, "chain": chain,
            "dest_address": dest_address, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn bridge_unwrap(
        &self, account: &str, wrapped_tx_hash: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/bridge/unwrap", serde_json::json!({
            "account": account, "wrapped_tx_hash": wrapped_tx_hash,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn bridge_fund(
        &self, funder: &str, amount_hunits: u64, chain: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/bridge/fund", serde_json::json!({
            "funder": funder, "amount": amount_hunits,
            "chain": chain, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn bridge_unlock(
        &self, account: &str, claim_proof: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/bridge/unlock", serde_json::json!({
            "account": account, "claim_proof": claim_proof,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn bridge_status(&self, account: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/bridge/status/{}", account)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Oracle Feeds (P3-B) ───────────────────────────────────────────────────

    pub async fn oracle_feed_create(
        &self, account: &str, feed_id: &str, asset_pair: &str,
        update_interval_epochs: u64, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/oracle/feed/create", serde_json::json!({
            "account": account, "feed_id": feed_id, "asset_pair": asset_pair,
            "update_interval_epochs": update_interval_epochs, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn oracle_report(
        &self, account: &str, feed_id: &str, commit_hash: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/oracle/report", serde_json::json!({
            "account": account, "feed_id": feed_id,
            "commit_hash": commit_hash, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn oracle_feed_finalize(
        &self, account: &str, feed_id: &str,
        reveals: Vec<serde_json::Value>, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/oracle/feed/finalize", serde_json::json!({
            "account": account, "feed_id": feed_id,
            "reveals": reveals, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn oracle_feed_get(&self, feed_id: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/oracle/feed/{}", feed_id)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Session Market (P3-C) ─────────────────────────────────────────────────

    pub async fn session_listing_create(
        &self, account: &str, session_id: &str, price_hunits: u64,
        summary_hash: &str, turn_count: u32, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/session/listing/create", serde_json::json!({
            "account": account, "session_id": session_id, "price": price_hunits,
            "summary_hash": summary_hash, "turn_count": turn_count,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn session_listing_buy(
        &self, account: &str, listing_id: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx(&format!("/api/session/listing/buy/{}", listing_id), serde_json::json!({
            "account": account, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn session_listing_cancel(
        &self, account: &str, listing_id: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx(&format!("/api/session/listing/cancel/{}", listing_id), serde_json::json!({
            "account": account, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn session_listings(&self, account: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/session/listings?account={}", account)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Agent Sessions (P3-D) ─────────────────────────────────────────────────

    pub async fn agent_session_open(
        &self, client: &str, client_pubkey: &str, fee_escrow: u64,
        system_prompt: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/agent/session/open", serde_json::json!({
            "client": client, "client_pubkey": client_pubkey, "fee_escrow": fee_escrow,
            "system_prompt": system_prompt, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn agent_session_close(
        &self, client: &str, session_id: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/agent/session/close", serde_json::json!({
            "client": client, "session_id": session_id, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn agent_session_get(&self, session_id: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/agent/session/{}", session_id)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── VRF Beacon (P3-E) ─────────────────────────────────────────────────────

    pub async fn vrf_commit(
        &self, account: &str, commit_hash: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/vrf/commit", serde_json::json!({
            "account": account, "commit_hash": commit_hash,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn vrf_reveal(
        &self, account: &str, secret_hex: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/vrf/reveal", serde_json::json!({
            "account": account, "secret_hex": secret_hex,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn vrf_beacon(&self, epoch: u64) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/vrf/beacon/{}", epoch)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Name Auctions (P4-A) ──────────────────────────────────────────────────

    pub async fn name_auction_open(
        &self, account: &str, name: &str, duration_epochs: u64, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/auction/name/open", serde_json::json!({
            "account": account, "name": name,
            "duration_epochs": duration_epochs, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn name_auction_bid(
        &self, account: &str, auction_id: &str, amount_hunits: u64, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/auction/bid", serde_json::json!({
            "account": account, "auction_id": auction_id, "amount": amount_hunits,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn name_auction_settle(
        &self, account: &str, auction_id: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/auction/settle", serde_json::json!({
            "account": account, "auction_id": auction_id,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn name_auction_cancel(
        &self, account: &str, auction_id: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/auction/cancel", serde_json::json!({
            "account": account, "auction_id": auction_id,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn name_auction_get(&self, auction_id: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/auction/{}", auction_id)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Freeport Auctions (P4-B) ──────────────────────────────────────────────

    pub async fn freeport_auction_open(
        &self, account: &str, item_id: &str, item_type: &str,
        duration_epochs: u64, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/freeport/auction/open", serde_json::json!({
            "account": account, "item_id": item_id, "item_type": item_type,
            "duration_epochs": duration_epochs, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn freeport_auction_bid(
        &self, account: &str, auction_id: &str, amount_hunits: u64, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/freeport/auction/bid", serde_json::json!({
            "account": account, "auction_id": auction_id, "amount": amount_hunits,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn freeport_auction_settle(
        &self, account: &str, auction_id: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/freeport/auction/settle", serde_json::json!({
            "account": account, "auction_id": auction_id,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn freeport_auction_get(&self, auction_id: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/freeport/auction/{}", auction_id)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Private Authorization (P4-E) ──────────────────────────────────────────

    pub async fn private_auth_enroll(
        &self, account: &str, group_id: &str, threshold_m: u32, threshold_n: u32,
        members: Vec<String>, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/private-auth/enroll", serde_json::json!({
            "account": account, "group_id": group_id,
            "threshold_m": threshold_m, "threshold_n": threshold_n,
            "members": members, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn private_auth_approve(
        &self, account: &str, group_id: &str, target_tx: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/private-auth/approve", serde_json::json!({
            "account": account, "group_id": group_id, "target_tx": target_tx,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn private_auth_status(&self, group_id: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/private-auth/status/{}", group_id)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── RAG Service (P5-A) ────────────────────────────────────────────────────

    pub async fn rag_index(
        &self, account: &str, doc_id: &str, content: &str, nonce: u64, sig: &str,
    ) -> Result<serde_json::Value> {
        self.http.post(self.url("/api/rag/index"))
            .json(&serde_json::json!({
                "account": account, "doc_id": doc_id, "content": content,
                "nonce": nonce, "signature": sig,
            }))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn rag_query(&self, account: &str, query: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/rag/query?account={}&q={}", account, query)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn rag_delete(
        &self, account: &str, doc_id: &str, nonce: u64, sig: &str,
    ) -> Result<serde_json::Value> {
        self.http.delete(self.url(&format!("/api/rag/doc/{}", doc_id)))
            .json(&serde_json::json!({ "account": account, "nonce": nonce, "signature": sig }))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Memory Service (P5-B) ─────────────────────────────────────────────────

    pub async fn memory_set(
        &self, account: &str, key: &str, value: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/memory/set", serde_json::json!({
            "account": account, "key": key, "value": value,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn memory_get(&self, account: &str, key: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/memory/get/{}/{}", account, key)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn memory_delete(
        &self, account: &str, key: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/memory/del", serde_json::json!({
            "account": account, "key": key, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn memory_scan(&self, account: &str, prefix: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/memory/scan/{}?prefix={}", account, prefix)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Fine-Tune Routes (P5-C) ───────────────────────────────────────────────

    pub async fn finetune_post(
        &self, account: &str, base_model: &str, dataset_cid: &str,
        lora_rank: u32, max_fee: u64, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/finetune/job", serde_json::json!({
            "account": account, "base_model": base_model, "dataset_cid": dataset_cid,
            "lora_rank": lora_rank, "max_fee": max_fee, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn finetune_complete(
        &self, worker: &str, job_id: &str, adapter_cid: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/finetune/complete", serde_json::json!({
            "worker": worker, "job_id": job_id, "adapter_cid": adapter_cid,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn finetune_job_get(&self, job_id: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/finetune/job/{}", job_id)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn finetune_jobs_open(&self) -> Result<serde_json::Value> {
        self.http.get(self.url("/api/finetune/jobs/open"))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Computer-Use Routes (P5-D) ────────────────────────────────────────────

    pub async fn computer_use_post(
        &self, account: &str, task_json: serde_json::Value,
        max_fee: u64, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/computer-use/job", serde_json::json!({
            "account": account, "task_json": task_json,
            "max_fee": max_fee, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn computer_use_complete(
        &self, worker: &str, job_id: &str, result_json: serde_json::Value, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/computer-use/complete", serde_json::json!({
            "worker": worker, "job_id": job_id, "result_json": result_json,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn computer_use_job_get(&self, job_id: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/computer-use/job/{}", job_id)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn computer_use_jobs_open(&self) -> Result<serde_json::Value> {
        self.http.get(self.url("/api/computer-use/jobs/open"))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Blob Serve Proof (P5-E) ───────────────────────────────────────────────

    pub async fn blob_serve_proof(
        &self, node_id: &str, cid: &str, bytes_served: u64,
        epoch: u64, proof_hash: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/blob/serve-proof", serde_json::json!({
            "node_id": node_id, "cid": cid, "bytes_served": bytes_served,
            "epoch": epoch, "proof_hash": proof_hash, "nonce": nonce, "signature": sig,
        })).await
    }

    // ── Blob Bandwidth Metering (P5-F) ────────────────────────────────────────

    pub async fn blob_bandwidth_peek(&self, cid: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/blob/bandwidth/{}", cid)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Peer Commerce (P5-G) ──────────────────────────────────────────────────

    pub async fn peer_commerce_register(
        &self, account: &str, product_cid: &str, price_hunits: u64,
        description: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/peer-commerce/register", serde_json::json!({
            "account": account, "product_cid": product_cid, "price": price_hunits,
            "description": description, "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn peer_commerce_get(&self, account: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/peer-commerce/{}", account)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn peer_commerce_list(&self) -> Result<serde_json::Value> {
        self.http.get(self.url("/api/peer-commerce/list"))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Amber Pill (P5-H) ─────────────────────────────────────────────────────

    pub async fn amber_pill_mint(
        &self, account: &str, device_fingerprint: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/amber-pill/mint", serde_json::json!({
            "account": account, "device_fingerprint": device_fingerprint,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn amber_pill_get(&self, account: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/amber-pill/{}", account)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Gateway Resolver (P5-I) ───────────────────────────────────────────────

    pub async fn gateway_resolve(&self, shortcode: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/gateway/resolve/{}", shortcode)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Snapshot Replication (P5-J) ───────────────────────────────────────────

    pub async fn snapshot_save(
        &self, account: &str, slug: &str, cid: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/snapshot/save", serde_json::json!({
            "account": account, "slug": slug, "cid": cid,
            "nonce": nonce, "signature": sig,
        })).await
    }

    pub async fn snapshot_get(&self, account: &str, slug: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/snapshot/{}/{}", account, slug)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn snapshot_list(&self, account: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/snapshot/list/{}", account)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Phone Storage (P5-K) ──────────────────────────────────────────────────

    pub async fn phone_storage_proof(
        &self, account: &str, device_id: &str, cid: &str,
        bytes_stored: u64, proof_hash: &str, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/phone-storage/proof", serde_json::json!({
            "account": account, "device_id": device_id, "cid": cid,
            "bytes_stored": bytes_stored, "proof_hash": proof_hash,
            "nonce": nonce, "signature": sig,
        })).await
    }

    // ── Node Health (P1-H) ────────────────────────────────────────────────────

    pub async fn health_detailed(&self) -> Result<serde_json::Value> {
        self.http.get(self.url("/api/node/health/detailed"))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Agent registry ────────────────────────────────────────────────────────

    pub async fn agent_register(&self, account: &str, name: &str, description: &str,
        tools: Vec<String>, model: &str, min_fee: u64, nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/agent/register", serde_json::json!({
            "account": account, "name": name, "description": description,
            "tools": tools, "model": model, "min_fee": min_fee,
            "nonce": nonce, "signed_by": account, "signature": sig,
        })).await
    }

    pub async fn agent_deregister(&self, account: &str, nonce: u64, sig: &str) -> Result<TxResponse> {
        self.post_tx("/api/agent/deregister", serde_json::json!({
            "account": account, "nonce": nonce, "signed_by": account, "signature": sig,
        })).await
    }

    pub async fn get_agent_registry(&self, tool: Option<&str>) -> Result<serde_json::Value> {
        let url = if let Some(t) = tool {
            self.url(&format!("/api/agent/registry?tool={}", t))
        } else {
            self.url("/api/agent/registry")
        };
        self.http.get(url).send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn get_agent_registry_entry(&self, account: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/agent/registry/{}", account)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    // ── Agentic task marketplace ──────────────────────────────────────────────

    pub async fn agent_credit_deposit(&self, account: &str, amount: u64, nonce: u64, sig: &str) -> Result<TxResponse> {
        self.post_tx("/api/agent/credit/deposit", serde_json::json!({
            "account": account, "amount": amount,
            "nonce": nonce, "signed_by": account, "signature": sig,
        })).await
    }

    pub async fn agent_credit_withdraw(&self, account: &str, amount: u64, nonce: u64, sig: &str) -> Result<TxResponse> {
        self.post_tx("/api/agent/credit/withdraw", serde_json::json!({
            "account": account, "amount": amount,
            "nonce": nonce, "signed_by": account, "signature": sig,
        })).await
    }

    pub async fn get_agent_credit(&self, account: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/agent/credit/{}", account)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn agent_task_post(&self, task_id: &str, requester: &str, description: &str,
        tools: Vec<String>, max_fee: u64, min_verifiers: u32,
        bid_window_epochs: u64, deadline_epoch: Option<u64>,
        nonce: u64, sig: &str,
    ) -> Result<TxResponse> {
        self.post_tx("/api/agent/task/post", serde_json::json!({
            "task_id": task_id, "requester": requester, "description": description,
            "tools_allowed": tools, "max_fee": max_fee, "min_verifiers": min_verifiers,
            "bid_window_epochs": bid_window_epochs, "deadline_epoch": deadline_epoch,
            "nonce": nonce, "signed_by": requester, "signature": sig,
        })).await
    }

    pub async fn get_agent_task(&self, task_id: &str) -> Result<serde_json::Value> {
        self.http.get(self.url(&format!("/api/agent/task/{}", task_id)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn get_agent_tasks(&self, requester: Option<&str>, agent: Option<&str>, status: Option<&str>) -> Result<serde_json::Value> {
        let mut params = vec![];
        if let Some(r) = requester { params.push(format!("requester={}", r)); }
        if let Some(a) = agent { params.push(format!("agent={}", a)); }
        if let Some(s) = status { params.push(format!("status={}", s)); }
        let qs = if params.is_empty() { String::new() } else { format!("?{}", params.join("&")) };
        self.http.get(self.url(&format!("/api/agent/tasks{}", qs)))
            .send().await?.error_for_status()?.json().await.map_err(Into::into)
    }

    pub async fn agent_task_bid(&self, task_id: &str, agent: &str, proposed_fee: u64, nonce: u64, sig: &str) -> Result<TxResponse> {
        self.post_tx(&format!("/api/agent/task/{}/bid", task_id), serde_json::json!({
            "agent": agent, "proposed_fee": proposed_fee,
            "nonce": nonce, "signed_by": agent, "signature": sig,
        })).await
    }

    pub async fn agent_task_assign(&self, task_id: &str, agent: &str, fee: u64, nonce: u64, sig: &str, signed_by: &str) -> Result<TxResponse> {
        self.post_tx(&format!("/api/agent/task/{}/assign", task_id), serde_json::json!({
            "agent": agent, "fee": fee,
            "nonce": nonce, "signed_by": signed_by, "signature": sig,
        })).await
    }

    pub async fn agent_task_submit(&self, task_id: &str, agent: &str, result_hash: &str, output_cid: &str, nonce: u64, sig: &str) -> Result<TxResponse> {
        self.post_tx(&format!("/api/agent/task/{}/submit", task_id), serde_json::json!({
            "agent": agent, "result_hash": result_hash, "output_cid": output_cid,
            "nonce": nonce, "signed_by": agent, "signature": sig,
        })).await
    }

    pub async fn agent_task_verifier_commit(&self, task_id: &str, verifier: &str, commit_hash: &str, nonce: u64, sig: &str) -> Result<TxResponse> {
        self.post_tx(&format!("/api/agent/task/{}/verify/commit", task_id), serde_json::json!({
            "verifier": verifier, "commit_hash": commit_hash,
            "nonce": nonce, "signed_by": verifier, "signature": sig,
        })).await
    }

    pub async fn agent_task_verifier_reveal(&self, task_id: &str, verifier: &str, result_hash: &str, salt: &str, nonce: u64, sig: &str) -> Result<TxResponse> {
        self.post_tx(&format!("/api/agent/task/{}/verify/reveal", task_id), serde_json::json!({
            "verifier": verifier, "result_hash": result_hash, "salt": salt,
            "nonce": nonce, "signed_by": verifier, "signature": sig,
        })).await
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

// ── Key management ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeyFile {
    private_key_hex: String,
}

/// Simple ed25519 keypair wrapper for signing HONE payloads.
pub struct KeyPair {
    signing_key: SigningKey,
}

impl KeyPair {
    pub fn generate() -> Self {
        let mut rng = rand::rngs::OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        Self { signing_key }
    }

    pub fn from_bytes(secret_bytes: &[u8; 32]) -> Result<Self> {
        Ok(Self {
            signing_key: SigningKey::from_bytes(secret_bytes),
        })
    }

    pub fn from_hex(hex_str: &str) -> Result<Self> {
        let bytes = hex::decode(hex_str.trim())
            .context("invalid private key hex")?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| anyhow!("private key must be exactly 32 bytes"))?;
        Self::from_bytes(&arr)
    }

    /// Load key from a JSON file.
    ///
    /// File format:
    /// `{ "private_key_hex": "<64 hex chars>" }`
    ///
    /// If file does not exist, a new key is generated and persisted.
    pub fn from_file(path: &Path) -> Result<Self> {
        if path.exists() {
            let raw = fs::read_to_string(path)
                .with_context(|| format!("failed to read key file {}", path.display()))?;
            let parsed: KeyFile = serde_json::from_str(&raw)
                .with_context(|| format!("invalid key file JSON {}", path.display()))?;
            return Self::from_hex(&parsed.private_key_hex);
        }

        let key = Self::generate();
        key.save_to_file(path)?;
        Ok(key)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create key directory {}", parent.display()))?;
        }
        let payload = KeyFile {
            private_key_hex: self.private_key_hex(),
        };
        let json = serde_json::to_string_pretty(&payload)?;
        fs::write(path, json)
            .with_context(|| format!("failed to write key file {}", path.display()))?;
        Ok(())
    }

    pub fn private_key_hex(&self) -> String {
        hex::encode(self.signing_key.to_bytes())
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.signing_key.verifying_key().to_bytes())
    }

    pub fn sign_entry_json(&self, entry_json: &str) -> String {
        self.sign_bytes(entry_json.as_bytes())
    }

    pub fn sign_bytes(&self, bytes: &[u8]) -> String {
        let sig = self.signing_key.sign(bytes);
        hex::encode(sig.to_bytes())
    }
}

// ── Wallet: BIP39 mnemonic + multi-chain key derivation ──────────────────────

/// On-disk wallet identity file.
/// Contains ONLY public information: account name, HONE public key, and
/// chain addresses.  No private keys, no mnemonic, ever.
/// Private keys must be derived from the mnemonic at session time.
#[derive(Debug, Serialize, Deserialize)]
pub struct WalletFile {
    pub version: u8,
    /// HONE account name.
    pub account: String,
    /// Derived HONE posting public key (hex). Kept for older callers.
    #[serde(alias = "btcpc_public_key_hex")]
    pub hone_public_key_hex: String,
    /// HONE role public keys derived from the canonical HONE wallet path.
    #[serde(default, alias = "btcpc_role_public_keys")]
    pub hone_role_public_keys: std::collections::HashMap<String, String>,
    /// Derived chain addresses / public keys.
    /// Keys: "evm", "solana", "bitcoin". Values: address strings (all public).
    pub chain_addresses: std::collections::HashMap<String, String>,
}

/// Derivation paths used per chain.
pub mod paths {
    const H: u32 = 0x8000_0000;
    /// HONE SLIP-44 coin type. This must match hone-node wallet derivation.
    pub const HONE_COIN: u32 = 6942;
    /// HONE owner key: cold key for key rotation and governance.
    pub const HONE_OWNER: &[u32] = &[44 | H, HONE_COIN | H, 0 | H, 0 | H];
    /// HONE active key: transfers, staking, and API-key registration.
    pub const HONE_ACTIVE: &[u32] = &[44 | H, HONE_COIN | H, 1 | H, 0 | H];
    /// HONE posting key: daily protocol activity.
    pub const HONE_POSTING: &[u32] = &[44 | H, HONE_COIN | H, 2 | H, 0 | H];
    /// HONE memo key: encrypted messages and selective disclosure.
    pub const HONE_MEMO: &[u32] = &[44 | H, HONE_COIN | H, 3 | H, 0 | H];
    /// HONE hide key: private content encryption.
    pub const HONE_HIDE: &[u32] = &[44 | H, HONE_COIN | H, 4 | H, 0 | H];
    /// HONE seek key: encrypted buyer delivery.
    pub const HONE_SEEK: &[u32] = &[44 | H, HONE_COIN | H, 5 | H, 0 | H];
    /// Canonical HONE signing key for backwards-compatible callers: posting.
    pub const HONE: &[u32] = HONE_POSTING;
    /// Legacy SDK v1 path. Use only to migrate older public wallet files.
    pub const HONE_LEGACY: &[u32] = &[44 | H, 2301 | H, 0 | H, 0 | H, 0 | H];
    pub const HONE_OWNER_STR: &str = "m/44'/6942'/0'/0'";
    pub const HONE_ACTIVE_STR: &str = "m/44'/6942'/1'/0'";
    pub const HONE_POSTING_STR: &str = "m/44'/6942'/2'/0'";
    /// Ethereum/EVM secp256k1 BIP44.
    pub const EVM: &str = "m/44'/60'/0'/0/0";
    /// Solana ed25519 (SLIP10, all hardened).
    pub const SOLANA: &[u32] = &[0x8000002C, 0x800001F5, 0x80000000, 0x80000000];
    /// Bitcoin secp256k1 BIP44 (P2PKH).
    pub const BITCOIN: &str = "m/44'/0'/0'/0/0";
}

/// Ephemeral wallet built from a BIP39 mnemonic.
/// The mnemonic is held in memory only for the duration of wallet creation;
/// after `save_to_file` it must be discarded.
pub struct Wallet {
    /// Shown to the user once; never persisted by this library.
    pub mnemonic: bip39::Mnemonic,
    pub account: String,
    seed: Vec<u8>,
}

impl Wallet {
    /// Generate a fresh 12-word mnemonic wallet.
    pub fn generate(account: &str) -> Result<Self> {
        let mnemonic = bip39::Mnemonic::generate(12)
            .map_err(|e| anyhow!("mnemonic generation failed: {}", e))?;
        Ok(Self::from_mnemonic(mnemonic, account))
    }

    /// Restore an ephemeral wallet from a mnemonic phrase (for re-publishing addresses).
    pub fn from_phrase(phrase: &str, account: &str) -> Result<Self> {
        let mnemonic = bip39::Mnemonic::parse(phrase)
            .map_err(|e| anyhow!("invalid mnemonic: {}", e))?;
        Ok(Self::from_mnemonic(mnemonic, account))
    }

    fn from_mnemonic(mnemonic: bip39::Mnemonic, account: &str) -> Self {
        let seed = mnemonic.to_seed("").to_vec();
        Self { mnemonic, account: account.to_string(), seed }
    }

    /// Save the wallet identity file.
    /// Writes ONLY public information: account name, HONE public key, chain addresses.
    /// No private key, no mnemonic, no seed — those must be kept by the user.
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let kp = self.hone_keypair()?;
        let role_keys = self.hone_role_public_keys()?;
        let mut addrs = std::collections::HashMap::new();
        if let Ok(a) = self.evm_address()       { addrs.insert("evm".into(), a); }
        if let Ok(a) = self.bitcoin_pubkey_hex() { addrs.insert("bitcoin".into(), a); }
        addrs.insert("solana".into(), self.solana_address());

        let wf = WalletFile {
            version: 3,
            account: self.account.clone(),
            hone_public_key_hex: kp.public_key_hex(),
            hone_role_public_keys: role_keys,
            chain_addresses: addrs,
        };
        fs::write(path, serde_json::to_string_pretty(&wf)?)?;
        Ok(())
    }

    /// HONE signing key derived via SLIP10-ed25519.
    pub fn hone_keypair(&self) -> Result<KeyPair> {
        self.hone_role_keypair("posting")
    }

    /// HONE role key derived from canonical m/44'/6942'/role'/0' paths.
    pub fn hone_role_keypair(&self, role: &str) -> Result<KeyPair> {
        let path = match role {
            "owner" => paths::HONE_OWNER,
            "active" => paths::HONE_ACTIVE,
            "posting" => paths::HONE_POSTING,
            "memo" => paths::HONE_MEMO,
            "hide" => paths::HONE_HIDE,
            "seek" => paths::HONE_SEEK,
            other => return Err(anyhow!("unknown HONE wallet role '{}'", other)),
        };
        let key_bytes = slip10_ed25519_derive(&self.seed, path);
        KeyPair::from_bytes(&key_bytes)
    }

    /// Legacy SDK v1 HONE key path. Only for migration of existing wallet.json files.
    pub fn legacy_hone_keypair(&self) -> Result<KeyPair> {
        let key_bytes = slip10_ed25519_derive(&self.seed, paths::HONE_LEGACY);
        KeyPair::from_bytes(&key_bytes)
    }

    pub fn hone_role_public_keys(&self) -> Result<std::collections::HashMap<String, String>> {
        let mut out = std::collections::HashMap::new();
        for role in ["owner", "active", "posting", "memo", "hide", "seek"] {
            out.insert(role.to_string(), self.hone_role_keypair(role)?.public_key_hex());
        }
        Ok(out)
    }

    /// EVM address string (0x-prefixed) from BIP44 secp256k1.
    pub fn evm_address(&self) -> Result<String> {
        let key_bytes = bip32_secp256k1_derive(&self.seed, paths::EVM)?;
        let sk = k256::ecdsa::SigningKey::from_slice(&key_bytes)
            .map_err(|_| anyhow!("invalid secp256k1 key"))?;
        let vk = sk.verifying_key();
        // Keccak256 of the 64-byte uncompressed public key (strip 0x04 prefix byte).
        let uncompressed = vk.to_encoded_point(false);
        use sha3::Digest;
        let hash = sha3::Keccak256::digest(&uncompressed.as_bytes()[1..]);
        Ok(format!("0x{}", hex::encode(&hash[12..])))
    }

    /// Solana address (base58 of ed25519 pubkey) derived via SLIP10.
    pub fn solana_address(&self) -> String {
        let key_bytes = slip10_ed25519_derive(&self.seed, paths::SOLANA);
        let signing = ed25519_dalek::SigningKey::from_bytes(&key_bytes);
        bs58::encode(signing.verifying_key().to_bytes()).into_string()
    }

    /// Bitcoin secp256k1 compressed public key hex (BIP44).
    /// Full P2WPKH bech32 encoding is deferred; the compressed pubkey is stored.
    pub fn bitcoin_pubkey_hex(&self) -> Result<String> {
        let key_bytes = bip32_secp256k1_derive(&self.seed, paths::BITCOIN)?;
        let sk = k256::ecdsa::SigningKey::from_slice(&key_bytes)
            .map_err(|_| anyhow!("invalid secp256k1 key for bitcoin"))?;
        let compressed = sk.verifying_key().to_encoded_point(true);
        Ok(hex::encode(compressed.as_bytes()))
    }

    // ── External-chain PRIVATE keys (for full backup / import into other wallets) ──
    // These expose the private half the address methods above compute internally.
    // Handle with the same care as the mnemonic.

    /// EVM (Ethereum/Base/BSC/…) private key, 0x-prefixed 32-byte hex.
    /// Importable directly into MetaMask / any EVM wallet.
    pub fn evm_private_key_hex(&self) -> Result<String> {
        let key_bytes = bip32_secp256k1_derive(&self.seed, paths::EVM)?;
        Ok(format!("0x{}", hex::encode(key_bytes)))
    }

    /// Solana private key as base58 of the 64-byte ed25519 keypair
    /// (secret||public), the format Phantom / solana-keygen import expect.
    pub fn solana_private_key_base58(&self) -> String {
        let key_bytes = slip10_ed25519_derive(&self.seed, paths::SOLANA);
        let signing = ed25519_dalek::SigningKey::from_bytes(&key_bytes);
        let mut full = Vec::with_capacity(64);
        full.extend_from_slice(&signing.to_bytes());                    // 32-byte secret seed
        full.extend_from_slice(&signing.verifying_key().to_bytes());    // 32-byte public
        bs58::encode(full).into_string()
    }

    /// Bitcoin private key: raw 32-byte hex plus WIF (mainnet, compressed).
    /// Returns `(hex, wif)`.
    pub fn bitcoin_private_key(&self) -> Result<(String, String)> {
        let key_bytes = bip32_secp256k1_derive(&self.seed, paths::BITCOIN)?;
        let hex_key = hex::encode(key_bytes);
        // WIF: 0x80 || key || 0x01 (compressed) || base58check(4-byte double-sha256 checksum)
        let mut payload = Vec::with_capacity(38);
        payload.push(0x80);
        payload.extend_from_slice(&key_bytes);
        payload.push(0x01);
        use sha2::{Digest, Sha256};
        let checksum = Sha256::digest(Sha256::digest(&payload));
        payload.extend_from_slice(&checksum[..4]);
        let wif = bs58::encode(payload).into_string();
        Ok((hex_key, wif))
    }

    /// All derived chain addresses for `WalletFamilyPublish`.
    /// Returns `(chain, address, derivation_path)` tuples.
    pub fn chain_addresses(&self) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        if let Ok(kp) = self.hone_keypair() {
            out.push(("hone".into(), kp.public_key_hex(), paths::HONE_POSTING_STR.into()));
        }
        if let Ok(addr) = self.evm_address() {
            out.push(("evm".into(), addr, "m/44'/60'/0'/0/0".into()));
        }
        out.push(("solana".into(), self.solana_address(), "m/44'/501'/0'/0'".into()));
        if let Ok(pk) = self.bitcoin_pubkey_hex() {
            out.push(("bitcoin".into(), pk, "m/44'/0'/0'/0/0".into()));
        }
        out
    }
}

// ── SLIP10-ed25519 key derivation ─────────────────────────────────────────────

fn hmac_sha512(key: &[u8], data: &[u8]) -> [u8; 64] {
    use hmac::{Hmac, Mac};
    use sha2::Sha512;
    let mut mac = Hmac::<Sha512>::new_from_slice(key).expect("HMAC key error");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

/// Derive an ed25519 private key from `seed` using the SLIP10 scheme.
/// All path components must be hardened (bit 31 set).
fn slip10_ed25519_derive(seed: &[u8], path: &[u32]) -> [u8; 32] {
    let master = hmac_sha512(b"ed25519 seed", seed);
    let mut key: [u8; 32] = master[..32].try_into().unwrap();
    let mut chain: [u8; 32] = master[32..].try_into().unwrap();

    for &index in path {
        let mut data = [0u8; 37];
        data[0] = 0x00;
        data[1..33].copy_from_slice(&key);
        data[33..37].copy_from_slice(&index.to_be_bytes());
        let result = hmac_sha512(&chain, &data);
        key = result[..32].try_into().unwrap();
        chain = result[32..].try_into().unwrap();
    }
    key
}

// ── BIP32-secp256k1 key derivation ───────────────────────────────────────────

/// Derive a secp256k1 private key from `seed` using BIP32.
/// Path format: "m/44'/60'/0'/0/0". Apostrophe = hardened (index | 0x80000000).
fn bip32_secp256k1_derive(seed: &[u8], path: &str) -> Result<[u8; 32]> {
    use k256::elliptic_curve::PrimeField;
    use k256::Scalar;

    // Master key ("Bitcoin seed" is the canonical HMAC key for all BIP32 secp256k1 chains).
    let master = hmac_sha512(b"Bitcoin seed", seed);
    let mut key: [u8; 32] = master[..32].try_into().unwrap();
    let mut chain: [u8; 32] = master[32..].try_into().unwrap();

    let segments = path.trim_start_matches("m/").split('/');
    for seg in segments {
        if seg.is_empty() { continue; }
        let (idx_str, hardened) = if seg.ends_with('\'') {
            (&seg[..seg.len() - 1], true)
        } else {
            (seg, false)
        };
        let index: u32 = idx_str.parse().map_err(|_| anyhow!("bad path segment '{}'", seg))?;
        let child_index = if hardened { 0x8000_0000 | index } else { index };

        let data: Vec<u8> = if hardened {
            let mut d = vec![0x00u8];
            d.extend_from_slice(&key);
            d.extend_from_slice(&child_index.to_be_bytes());
            d
        } else {
            let sk = k256::ecdsa::SigningKey::from_slice(&key)
                .map_err(|_| anyhow!("invalid key at unhardened step"))?;
            let compressed = sk.verifying_key().to_encoded_point(true);
            let mut d = compressed.as_bytes().to_vec();
            d.extend_from_slice(&child_index.to_be_bytes());
            d
        };

        let result = hmac_sha512(&chain, &data);
        let il = &result[..32];
        chain = result[32..].try_into().unwrap();

        // child_key = (parent_key + il) mod n.
        let key_fb = k256::FieldBytes::clone_from_slice(&key);
        let il_fb = k256::FieldBytes::clone_from_slice(il);
        let parent = Option::<Scalar>::from(Scalar::from_repr(key_fb))
            .ok_or_else(|| anyhow!("parent scalar invalid"))?;
        let tweak = Option::<Scalar>::from(Scalar::from_repr(il_fb))
            .ok_or_else(|| anyhow!("IL scalar invalid"))?;
        let child = parent + tweak;
        if bool::from(child.is_zero()) {
            anyhow::bail!("derived a zero child key — try a different index");
        }
        let child_bytes = child.to_repr();
        key.copy_from_slice(child_bytes.as_slice());
    }

    Ok(key)
}

// ── BSP contract wrappers ─────────────────────────────────────────────────────

const DEFAULT_CONTRACT_GAS: u64 = 300_000_000_000;

pub struct Bsp20Client<'a> {
    client: &'a HoneClient,
    contract_id: String,
}

impl<'a> Bsp20Client<'a> {
    pub fn new(client: &'a HoneClient, contract_id: &str) -> Self {
        Self {
            client,
            contract_id: contract_id.to_string(),
        }
    }

    pub async fn name(&self) -> Result<String> {
        let resp = self
            .client
            .contract_view(&self.contract_id, "ft_name", serde_json::Value::Null, DEFAULT_CONTRACT_GAS)
            .await?;
        parse_contract_result_string(resp)
    }

    pub async fn symbol(&self) -> Result<String> {
        let resp = self
            .client
            .contract_view(&self.contract_id, "ft_symbol", serde_json::Value::Null, DEFAULT_CONTRACT_GAS)
            .await?;
        parse_contract_result_string(resp)
    }

    pub async fn total_supply(&self) -> Result<u64> {
        let resp = self
            .client
            .contract_view(
                &self.contract_id,
                "ft_total_supply",
                serde_json::Value::Null,
                DEFAULT_CONTRACT_GAS,
            )
            .await?;
        parse_contract_result_u64(resp)
    }

    pub async fn balance_of(&self, account: &str) -> Result<u64> {
        let resp = self
            .client
            .contract_view(
                &self.contract_id,
                "ft_balance_of",
                serde_json::json!({ "account_id": account }),
                DEFAULT_CONTRACT_GAS,
            )
            .await?;
        parse_contract_result_u64(resp)
    }

    pub async fn transfer(
        &self,
        to: &str,
        amount: u64,
        signer: &str,
        keypair: &KeyPair,
    ) -> Result<serde_json::Value> {
        let nonce = self.client.next_nonce(signer).await?;
        let epoch = self.client.latest().await?.current_epoch;
        let msg = contract_call_message(signer, &self.contract_id, "ft_transfer", nonce, epoch);
        let signature = keypair.sign_bytes(&serde_json::to_vec(&msg)?);

        self.client
            .contract_call(
                &self.contract_id,
                "ft_transfer",
                serde_json::json!({ "receiver_id": to, "amount": amount }),
                signer,
                0,
                DEFAULT_CONTRACT_GAS,
                nonce,
                &signature,
            )
            .await
    }

    pub async fn approve(
        &self,
        spender: &str,
        amount: u64,
        signer: &str,
        keypair: &KeyPair,
    ) -> Result<serde_json::Value> {
        let nonce = self.client.next_nonce(signer).await?;
        let epoch = self.client.latest().await?.current_epoch;
        let msg = contract_call_message(signer, &self.contract_id, "ft_approve", nonce, epoch);
        let signature = keypair.sign_bytes(&serde_json::to_vec(&msg)?);

        self.client
            .contract_call(
                &self.contract_id,
                "ft_approve",
                serde_json::json!({ "spender_id": spender, "amount": amount }),
                signer,
                0,
                DEFAULT_CONTRACT_GAS,
                nonce,
                &signature,
            )
            .await
    }
}

pub struct Bsp721Client<'a> {
    client: &'a HoneClient,
    contract_id: String,
}

impl<'a> Bsp721Client<'a> {
    pub fn new(client: &'a HoneClient, contract_id: &str) -> Self {
        Self {
            client,
            contract_id: contract_id.to_string(),
        }
    }

    pub async fn name(&self) -> Result<String> {
        let resp = self
            .client
            .contract_view(&self.contract_id, "nft_name", serde_json::Value::Null, DEFAULT_CONTRACT_GAS)
            .await?;
        parse_contract_result_string(resp)
    }

    pub async fn total_supply(&self) -> Result<u64> {
        let resp = self
            .client
            .contract_view(
                &self.contract_id,
                "nft_total_supply",
                serde_json::Value::Null,
                DEFAULT_CONTRACT_GAS,
            )
            .await?;
        parse_contract_result_u64(resp)
    }

    pub async fn owner_of(&self, token_id: u64) -> Result<String> {
        let resp = self
            .client
            .contract_view(
                &self.contract_id,
                "nft_token",
                serde_json::json!({ "token_id": token_id.to_string() }),
                DEFAULT_CONTRACT_GAS,
            )
            .await?;
        parse_contract_result_string(resp)
    }

    pub async fn token_uri(&self, token_id: u64) -> Result<String> {
        let resp = self
            .client
            .contract_view(
                &self.contract_id,
                "nft_token_metadata",
                serde_json::json!({ "token_id": token_id.to_string() }),
                DEFAULT_CONTRACT_GAS,
            )
            .await?;
        parse_contract_result_string(resp)
    }

    pub async fn transfer_nft(
        &self,
        to: &str,
        token_id: u64,
        signer: &str,
        keypair: &KeyPair,
    ) -> Result<serde_json::Value> {
        let nonce = self.client.next_nonce(signer).await?;
        let epoch = self.client.latest().await?.current_epoch;
        let msg = contract_call_message(signer, &self.contract_id, "nft_transfer", nonce, epoch);
        let signature = keypair.sign_bytes(&serde_json::to_vec(&msg)?);

        self.client
            .contract_call(
                &self.contract_id,
                "nft_transfer",
                serde_json::json!({ "receiver_id": to, "token_id": token_id.to_string() }),
                signer,
                0,
                DEFAULT_CONTRACT_GAS,
                nonce,
                &signature,
            )
            .await
    }
}

// ── Utility helpers ───────────────────────────────────────────────────────────

/// Convert HONE decimal string to hunits (u64).
pub fn hone_to_hunits(hone: &str) -> Result<u64> {
    parse_decimal_hone_to_hunits(hone)
}

/// Convert hunits (u64) to HONE decimal string with 10 fractional digits.
pub fn hunits_to_hone(hunits: u64) -> String {
    let whole = hunits / HUNITS_PER_HONE;
    let frac = hunits % HUNITS_PER_HONE;
    format!("{}.{:010}", whole, frac)
}

fn contract_call_message(
    signer: &str,
    contract_id: &str,
    method: &str,
    nonce: u64,
    epoch: u64,
) -> serde_json::Value {
    serde_json::json!({
        "type": "CONTRACT_CALL",
        "signer": signer,
        "contract_id": contract_id,
        "method": method,
        "nonce": nonce,
        "epoch": epoch,
    })
}

fn parse_contract_result_string(resp: serde_json::Value) -> Result<String> {
    let value = extract_contract_result(resp)?;
    match value {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        other => Err(anyhow!("expected string contract result, got {}", other)),
    }
}

fn parse_contract_result_u64(resp: serde_json::Value) -> Result<u64> {
    let value = extract_contract_result(resp)?;
    parse_hunits_value(&value)
}

fn extract_contract_result(resp: serde_json::Value) -> Result<serde_json::Value> {
    if let Some(false) = resp.get("ok").and_then(|v| v.as_bool()) {
        let err = resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown contract error");
        return Err(anyhow!("contract call failed: {}", err));
    }
    if let Some(result) = resp.get("result") {
        return Ok(result.clone());
    }
    Ok(resp)
}

fn parse_hunits_value(value: &serde_json::Value) -> Result<u64> {
    match value {
        serde_json::Value::Number(n) => parse_number_string_to_hunits(&n.to_string()),
        serde_json::Value::String(s) => parse_number_string_to_hunits(s),
        other => Err(anyhow!("expected numeric value, got {}", other)),
    }
}

fn parse_number_string_to_hunits(raw: &str) -> Result<u64> {
    let trimmed = raw.trim();
    if trimmed.contains('.') {
        parse_decimal_hone_to_hunits(trimmed)
    } else {
        trimmed
            .parse::<u64>()
            .map_err(|e| anyhow!("invalid integer amount '{}': {}", trimmed, e))
    }
}

fn parse_decimal_hone_to_hunits(s: &str) -> Result<u64> {
    let s = s.trim();
    let (int_str, frac_str) = match s.find('.') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    };

    let int_part = if int_str.is_empty() {
        0u64
    } else {
        int_str
            .parse::<u64>()
            .map_err(|e| anyhow!("invalid integer part '{}': {}", int_str, e))?
    };

    let frac_len = frac_str.len().min(10);
    let frac_digits = if frac_len == 0 {
        0u64
    } else {
        frac_str[..frac_len]
            .parse::<u64>()
            .map_err(|e| anyhow!("invalid fractional part '{}': {}", frac_str, e))?
    };
    let frac_part = frac_digits * 10u64.pow((10 - frac_len) as u32);

    int_part
        .checked_mul(HUNITS_PER_HONE)
        .and_then(|v| v.checked_add(frac_part))
        .ok_or_else(|| anyhow!("amount overflow"))
}

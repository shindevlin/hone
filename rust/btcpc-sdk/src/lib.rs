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

use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, path::Path};

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
    /// Balance in dreams (smallest unit).
    pub dreams: u64,
    pub token: String,
}

/// Response from `GET /api/stake/:account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StakeResponse {
    pub account: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AllBalancesResponse {
    balances: HashMap<String, serde_json::Value>,
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

    /// Fetch all token balances of `account` as dreams (u64).
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
            out.insert(token, parse_dreams_value(&value)?);
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
        parse_dreams_value(nonce_value)
    }

    /// Return the next valid nonce (`current + 1`) for an account.
    pub async fn next_nonce(&self, account: &str) -> Result<u64> {
        let current = self.get_nonce(account).await?;
        current
            .checked_add(1)
            .ok_or_else(|| anyhow!("nonce overflow for account '{}'", account))
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
        amount_dreams: u64,
        memo: Option<&str>,
        nonce: u64,
        sig: &str,
    ) -> Result<TxResponse> {
        let body = serde_json::json!({
            "from": from,
            "to": to,
            "amount": amount_dreams,
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
        amount_dreams: u64,
        nonce: u64,
        sig: &str,
    ) -> Result<TxResponse> {
        let body = serde_json::json!({
            "account": account,
            "amount": amount_dreams,
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
        amount_dreams: u64,
        nonce: u64,
        sig: &str,
    ) -> Result<TxResponse> {
        let body = serde_json::json!({
            "account": account,
            "amount": amount_dreams,
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
        deposit_dreams: u64,
        gas: u64,
        nonce: u64,
        signature: &str,
    ) -> Result<serde_json::Value> {
        let body = serde_json::json!({
            "contract_id": contract_id,
            "method": method,
            "args": args,
            "signer": signer,
            "deposit": deposit_dreams,
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

/// Simple ed25519 keypair wrapper for signing BTCPC payloads.
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

// ── BSP contract wrappers ─────────────────────────────────────────────────────

const DEFAULT_CONTRACT_GAS: u64 = 300_000_000_000;

pub struct Bsp20Client<'a> {
    client: &'a BtcpcClient,
    contract_id: String,
}

impl<'a> Bsp20Client<'a> {
    pub fn new(client: &'a BtcpcClient, contract_id: &str) -> Self {
        Self {
            client,
            contract_id: contract_id.to_string(),
        }
    }

    pub async fn name(&self) -> Result<String> {
        let resp = self
            .client
            .contract_view(&self.contract_id, "name", serde_json::Value::Null, DEFAULT_CONTRACT_GAS)
            .await?;
        parse_contract_result_string(resp)
    }

    pub async fn symbol(&self) -> Result<String> {
        let resp = self
            .client
            .contract_view(&self.contract_id, "symbol", serde_json::Value::Null, DEFAULT_CONTRACT_GAS)
            .await?;
        parse_contract_result_string(resp)
    }

    pub async fn total_supply(&self) -> Result<u64> {
        let resp = self
            .client
            .contract_view(
                &self.contract_id,
                "total_supply",
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
                "balance_of",
                serde_json::json!({ "account": account }),
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
        let msg = contract_call_message(signer, &self.contract_id, "transfer", nonce, epoch);
        let signature = keypair.sign_bytes(&serde_json::to_vec(&msg)?);

        self.client
            .contract_call(
                &self.contract_id,
                "transfer",
                serde_json::json!({ "to": to, "amount": amount }),
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
        let msg = contract_call_message(signer, &self.contract_id, "approve", nonce, epoch);
        let signature = keypair.sign_bytes(&serde_json::to_vec(&msg)?);

        self.client
            .contract_call(
                &self.contract_id,
                "approve",
                serde_json::json!({ "spender": spender, "amount": amount }),
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
    client: &'a BtcpcClient,
    contract_id: String,
}

impl<'a> Bsp721Client<'a> {
    pub fn new(client: &'a BtcpcClient, contract_id: &str) -> Self {
        Self {
            client,
            contract_id: contract_id.to_string(),
        }
    }

    pub async fn name(&self) -> Result<String> {
        let resp = self
            .client
            .contract_view(&self.contract_id, "name", serde_json::Value::Null, DEFAULT_CONTRACT_GAS)
            .await?;
        parse_contract_result_string(resp)
    }

    pub async fn total_supply(&self) -> Result<u64> {
        let resp = self
            .client
            .contract_view(
                &self.contract_id,
                "total_supply",
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
                "owner_of",
                serde_json::json!({ "token_id": token_id }),
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
                "token_uri",
                serde_json::json!({ "token_id": token_id }),
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
        let msg = contract_call_message(signer, &self.contract_id, "transfer", nonce, epoch);
        let signature = keypair.sign_bytes(&serde_json::to_vec(&msg)?);

        self.client
            .contract_call(
                &self.contract_id,
                "transfer",
                serde_json::json!({ "to": to, "token_id": token_id }),
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

/// Convert BTCPC decimal string to dreams (u64).
pub fn btcpc_to_dreams(btcpc: &str) -> Result<u64> {
    parse_decimal_btcpc_to_dreams(btcpc)
}

/// Convert dreams (u64) to BTCPC decimal string with 10 fractional digits.
pub fn dreams_to_btcpc(dreams: u64) -> String {
    let whole = dreams / DREAMS_PER_BTCPC;
    let frac = dreams % DREAMS_PER_BTCPC;
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
    parse_dreams_value(&value)
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

fn parse_dreams_value(value: &serde_json::Value) -> Result<u64> {
    match value {
        serde_json::Value::Number(n) => parse_number_string_to_dreams(&n.to_string()),
        serde_json::Value::String(s) => parse_number_string_to_dreams(s),
        other => Err(anyhow!("expected numeric value, got {}", other)),
    }
}

fn parse_number_string_to_dreams(raw: &str) -> Result<u64> {
    let trimmed = raw.trim();
    if trimmed.contains('.') {
        parse_decimal_btcpc_to_dreams(trimmed)
    } else {
        trimmed
            .parse::<u64>()
            .map_err(|e| anyhow!("invalid integer amount '{}': {}", trimmed, e))
    }
}

fn parse_decimal_btcpc_to_dreams(s: &str) -> Result<u64> {
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
        .checked_mul(DREAMS_PER_BTCPC)
        .and_then(|v| v.checked_add(frac_part))
        .ok_or_else(|| anyhow!("amount overflow"))
}

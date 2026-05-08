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
    /// Bearer token sent in `Authorization: Bearer <api_key>` for paid endpoints.
    /// Currently equals your account name; rotate-able tokens land in a future release.
    api_key: Option<String>,
    http: reqwest::Client,
}

impl BtcpcClient {
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
    /// Currently the API key is your account name. Set via `BTCPC_API_KEY` in env.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Build a client from environment variables:
    /// - `BTCPC_API_URL`   — defaults to [`DEFAULT_API_URL`]
    /// - `BTCPC_ACCOUNT`   — optional default account name
    /// - `BTCPC_API_KEY`   — API key for paid endpoints (currently equals account name)
    pub fn from_env() -> Self {
        let base_url = std::env::var("BTCPC_API_URL")
            .unwrap_or_else(|_| DEFAULT_API_URL.to_owned());
        let account = std::env::var("BTCPC_ACCOUNT").ok();
        // BTCPC_API_KEY is the Bearer token for paid calls. Falls back to BTCPC_ACCOUNT
        // so callers that only set one variable still work.
        let api_key = std::env::var("BTCPC_API_KEY").ok()
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

    // ── Inference ─────────────────────────────────────────────────────────────

    /// Send an OpenAI-compatible chat completion request to the BTCPC inference
    /// gateway at `/v1/chat/completions`.
    ///
    /// Requires `BTCPC_API_KEY` (or `BTCPC_ACCOUNT`) to be set — the fee of
    /// 10 000 dreams (0.0001 BTCPC) is debited per call.
    ///
    /// `model` defaults to the node's active model when `None`.
    pub async fn chat_completions(
        &self,
        messages: Vec<serde_json::Value>,
        model: Option<&str>,
    ) -> Result<serde_json::Value> {
        let key = self.api_key.as_deref()
            .ok_or_else(|| anyhow!("BTCPC_API_KEY not set — cannot call paid inference endpoint"))?;

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

// ── Wallet: BIP39 mnemonic + multi-chain key derivation ──────────────────────

/// On-disk wallet identity file.
/// Contains ONLY public information: account name, BTCPC public key, and
/// chain addresses.  No private keys, no mnemonic, ever.
/// Private keys must be derived from the mnemonic at session time.
#[derive(Debug, Serialize, Deserialize)]
pub struct WalletFile {
    pub version: u8,
    /// BTCPC account name.
    pub account: String,
    /// Derived BTCPC ed25519 public key (hex). Used to verify identity.
    pub btcpc_public_key_hex: String,
    /// Derived chain addresses / public keys.
    /// Keys: "evm", "solana", "bitcoin". Values: address strings (all public).
    pub chain_addresses: std::collections::HashMap<String, String>,
}

/// Derivation paths used per chain.
pub mod paths {
    /// BTCPC ed25519 (SLIP10, all components hardened). Coin type 2301.
    pub const BTCPC: &[u32] = &[0x8000002C, 0x800008FD, 0x80000000, 0x80000000, 0x80000000];
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
    /// Writes ONLY public information: account name, BTCPC public key, chain addresses.
    /// No private key, no mnemonic, no seed — those must be kept by the user.
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let kp = self.btcpc_keypair()?;
        let mut addrs = std::collections::HashMap::new();
        if let Ok(a) = self.evm_address()       { addrs.insert("evm".into(), a); }
        if let Ok(a) = self.bitcoin_pubkey_hex() { addrs.insert("bitcoin".into(), a); }
        addrs.insert("solana".into(), self.solana_address());

        let wf = WalletFile {
            version: 2,
            account: self.account.clone(),
            btcpc_public_key_hex: kp.public_key_hex(),
            chain_addresses: addrs,
        };
        fs::write(path, serde_json::to_string_pretty(&wf)?)?;
        Ok(())
    }

    /// BTCPC signing key derived via SLIP10-ed25519.
    pub fn btcpc_keypair(&self) -> Result<KeyPair> {
        let key_bytes = slip10_ed25519_derive(&self.seed, paths::BTCPC);
        KeyPair::from_bytes(&key_bytes)
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

    /// All derived chain addresses for `WalletFamilyPublish`.
    /// Returns `(chain, address, derivation_path)` tuples.
    pub fn chain_addresses(&self) -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        if let Ok(kp) = self.btcpc_keypair() {
            out.push(("btcpc".into(), kp.public_key_hex(), "m/44'/2301'/0'/0'/0'".into()));
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

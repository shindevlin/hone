//! HTTP API server (Axum) — replaces Node.js btcpc-api.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use axum::{
    Router,
    routing::{get, post},
    extract::{Path, State},
    Json, http::StatusCode,
};
use serde::{Deserialize, Deserializer};
use tower_http::cors::CorsLayer;
use tokio::sync::broadcast;
use btcpc_types::{Block, LedgerEntry, NATIVE_TOKEN, DREAMS_PER_BTCPC, TESTNET_CHAIN_ID};

/// Gossip envelope: entry JSON + out-of-band signature so peers can re-verify.
pub type GossipEntry = (LedgerEntry, Option<String>);

use crate::chain::Chain;
use crate::contracts::ContractEngine;
use crate::inference;
use crate::tx;

#[derive(Clone)]
pub struct AppState {
    pub chain: Arc<Chain>,
    pub contracts: Arc<ContractEngine>,
    /// Broadcast channel for gossiping newly accepted entries to the net module.
    /// Carries (entry, optional_signature) so the sig propagates with the gossip.
    pub tx_broadcast: broadcast::Sender<GossipEntry>,
    /// Faucet rate-limiter: account → last claim time (testnet only).
    pub faucet_claims: Arc<parking_lot::Mutex<HashMap<String, Instant>>>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        // ── GET endpoints ────────────────────────────────────────────────
        .route("/api/balance/:account", get(get_balance))
        .route("/api/balances/:account", get(get_all_balances))
        .route("/api/account/:account", get(get_account))
        .route("/api/block/:epoch", get(get_block))
        .route("/api/latest", get(get_latest))
        .route("/api/stake/:account", get(get_stake))
        .route("/api/epoch/:epoch", get(get_epoch))
        .route("/health", get(health))
        // ── POST endpoints ────────────────────────────────────────────────
        .route("/api/transfer", post(post_transfer))
        .route("/api/stake", post(post_stake))
        .route("/api/unstake", post(post_unstake))
        .route("/api/account/create", post(post_account_create))
        .route("/api/account/update-key", post(post_account_update_key))
        // ── Contract endpoints ────────────────────────────────────────────
        .route("/api/contract/deploy", post(post_contract_deploy))
        .route("/api/contract/call", post(post_contract_call))
        .route("/api/contract/view", post(post_contract_view))
        // ── Inference marketplace ─────────────────────────────────────────
        .route("/api/inference/post", post(post_inference_job))
        .route("/api/inference/bid", post(post_inference_bid))
        .route("/api/inference/complete", post(post_inference_complete))
        .route("/api/inference/verify", post(post_inference_verify))
        .route("/api/inference/claim", post(post_inference_claim))
        .route("/api/inference/review", post(post_inference_review))
        .route("/api/inference/cancel", post(post_inference_cancel))
        .route("/api/inference/jobs", get(get_inference_jobs))
        .route("/api/inference/job/:id", get(get_inference_job))
        .route("/api/inference/reputation/:node", get(get_inference_reputation))
        // ── Faucet (testnet only) ─────────────────────────────────────────
        .route("/api/faucet/claim", post(post_faucet_claim))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// ── GET handlers (unchanged) ──────────────────────────────────────────────────

// GET /api/balance/:account  →  { account, balance, token }
async fn get_balance(
    State(s): State<AppState>,
    Path(account): Path<String>,
) -> Json<serde_json::Value> {
    let balance = s.chain.get_balance(&account, NATIVE_TOKEN);
    Json(serde_json::json!({
        "account": account,
        "balance": balance as f64 / DREAMS_PER_BTCPC as f64,
        "dreams": balance,
        "token": NATIVE_TOKEN,
    }))
}

// GET /api/balances/:account  →  all token balances
async fn get_all_balances(
    State(s): State<AppState>,
    Path(account): Path<String>,
) -> Json<serde_json::Value> {
    let balances = s.chain.store.scan_balances(&account);
    // Return dreams as integers to avoid f64 precision loss.
    // Include a display string for convenience.
    let entries: Vec<serde_json::Value> = balances.into_iter()
        .map(|(token, dreams)| serde_json::json!({
            "token": token,
            "dreams": dreams,
            "display": format!("{:.10}", dreams as f64 / DREAMS_PER_BTCPC as f64),
        }))
        .collect();
    Json(serde_json::json!({ "account": account, "balances": entries }))
}

// GET /api/account/:account
async fn get_account(
    State(s): State<AppState>,
    Path(account): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match s.chain.store.get_account(&account) {
        Ok(Some(data)) => Ok(Json(data)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// GET /api/block/:epoch
async fn get_block(
    State(s): State<AppState>,
    Path(epoch): Path<u64>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match s.chain.store.read_block(epoch) {
        Ok(Some(data)) => {
            let block = Block::from_bytes(&data).ok_or(StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(Json(serde_json::json!({
                "epoch": epoch,
                "hash": block.header.hash_hex(),
                "header": block.header,
                "payload": block.payload,
            })))
        }
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// GET /api/latest
async fn get_latest(State(s): State<AppState>) -> Json<serde_json::Value> {
    let epoch = s.chain.store.latest_epoch().unwrap_or(0);
    let hash = s.chain.store.read_block(epoch)
        .ok().flatten()
        .and_then(|d| Block::from_bytes(&d))
        .map(|b| b.header.hash_hex())
        .unwrap_or_default();

    Json(serde_json::json!({
        "epoch": epoch,
        "hash": hash,
        "current_epoch": s.chain.current_epoch(),
    }))
}

// GET /api/stake/:account
async fn get_stake(
    State(s): State<AppState>,
    Path(account): Path<String>,
) -> Json<serde_json::Value> {
    let stake = s.chain.get_stake(&account);
    Json(serde_json::json!({
        "account": account,
        "stake": stake as f64 / DREAMS_PER_BTCPC as f64,
        "dreams": stake,
    }))
}

// GET /api/epoch/:epoch
async fn get_epoch(
    State(s): State<AppState>,
    Path(epoch): Path<u64>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match s.chain.store.get_epoch_meta(epoch) {
        Ok(Some(meta)) => Ok(Json(meta)),
        Ok(None) => {
            // Return basic info if no metadata yet
            Ok(Json(serde_json::json!({
                "epoch": epoch,
                "has_block": s.chain.store.has_block(epoch),
                "finalized": false,
            })))
        }
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// GET /health
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok", "node": "btcpc-node" }))
}

// ── Amount deserializer ───────────────────────────────────────────────────────
//
// Accepts three wire formats to eliminate f64 precision loss on large amounts:
//   integer  → treated as dreams directly          e.g. 15_000_000_000
//   float    → BTCPC × DREAMS_PER_BTCPC, rounded   e.g. 1.5   (only safe < ~900k BTCPC)
//   string   → decimal BTCPC, integer arithmetic   e.g. "1.5" (always safe)
//
// String form is preferred for any amount that might exceed 900_000 BTCPC.

fn deserialize_amount_dreams<'de, D>(d: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, Unexpected, Visitor};
    use std::fmt;

    struct AmountVisitor;

    impl<'de> Visitor<'de> for AmountVisitor {
        type Value = u64;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a numeric amount (BTCPC as float/string, or dreams as integer)")
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<u64, E> {
            Ok(v)
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<u64, E> {
            if v < 0 {
                Err(E::invalid_value(Unexpected::Signed(v), &self))
            } else {
                Ok(v as u64)
            }
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<u64, E> {
            if v < 0.0 {
                return Err(E::invalid_value(Unexpected::Float(v), &self));
            }
            Ok((v * DREAMS_PER_BTCPC as f64).round() as u64)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<u64, E> {
            parse_btcpc_str(v).map_err(|_| E::invalid_value(Unexpected::Str(v), &self))
        }
    }

    d.deserialize_any(AmountVisitor)
}

/// Parse a decimal BTCPC string to dreams using integer arithmetic only.
fn parse_btcpc_str(s: &str) -> Result<u64, ()> {
    let s = s.trim();
    let (int_str, frac_str) = match s.find('.') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    };
    let int_val: u64 = if int_str.is_empty() {
        0
    } else {
        int_str.parse().map_err(|_| ())?
    };
    // Take up to 10 fractional digits; pad with zeros on the right.
    let frac_len = frac_str.len().min(10);
    let frac_digits: u64 = if frac_len == 0 {
        0
    } else {
        frac_str[..frac_len].parse().map_err(|_| ())?
    };
    let frac_val = frac_digits * 10u64.pow((10 - frac_len) as u32);
    int_val
        .checked_mul(DREAMS_PER_BTCPC)
        .and_then(|v| v.checked_add(frac_val))
        .ok_or(())
}

// ── POST request bodies ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TransferBody {
    from: String,
    to: String,
    /// Amount: BTCPC as float/string, or dreams as integer.
    #[serde(deserialize_with = "deserialize_amount_dreams")]
    amount: u64,
    #[serde(default = "default_token")]
    token: String,
    memo: Option<String>,
    signed_by: String,
    nonce: u64,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct StakeBody {
    account: String,
    /// Amount: BTCPC as float/string, or dreams as integer.
    #[serde(deserialize_with = "deserialize_amount_dreams")]
    amount: u64,
    nonce: u64,
    signed_by: String,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct UnstakeBody {
    account: String,
    /// Amount: BTCPC as float/string, or dreams as integer.
    #[serde(deserialize_with = "deserialize_amount_dreams")]
    amount: u64,
    nonce: u64,
    signed_by: String,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct AccountCreateBody {
    account: String,
    /// Role-keyed map of hex-encoded ed25519 public keys (optional for watch-only accounts).
    #[serde(default)]
    keys: Option<std::collections::HashMap<String, String>>,
}

fn default_token() -> String {
    NATIVE_TOKEN.to_owned()
}

// ── POST handlers ─────────────────────────────────────────────────────────────

/// POST /api/transfer
/// Body: { "from", "to", "amount", "token", "memo", "signed_by", "nonce", "signature" }
async fn post_transfer(
    State(s): State<AppState>,
    Json(body): Json<TransferBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();
    let entry = LedgerEntry::Transfer {
        from: body.from,
        to: body.to,
        amount: body.amount,
        token: body.token,
        memo: body.memo,
        epoch,
        signed_by: body.signed_by,
        nonce: body.nonce,
    };
    let sig = non_empty(&body.signature);
    apply_and_broadcast(&s, entry, sig)
}

/// POST /api/stake
/// Body: { "account", "amount", "nonce", "signed_by", "signature" }
async fn post_stake(
    State(s): State<AppState>,
    Json(body): Json<StakeBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();
    let entry = LedgerEntry::Stake {
        account: body.account,
        amount: body.amount,
        epoch,
        nonce: body.nonce,
        signed_by: body.signed_by,
    };
    let sig = non_empty(&body.signature);
    apply_and_broadcast(&s, entry, sig)
}

/// POST /api/unstake
/// Body: { "account", "amount", "nonce", "signed_by", "signature" }
async fn post_unstake(
    State(s): State<AppState>,
    Json(body): Json<UnstakeBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();
    let entry = LedgerEntry::Unstake {
        account: body.account,
        amount: body.amount,
        epoch,
        nonce: body.nonce,
        signed_by: body.signed_by,
    };
    let sig = non_empty(&body.signature);
    apply_and_broadcast(&s, entry, sig)
}

/// POST /api/account/create
/// Body: { "account", "keys" (role->pubkey map, optional) }
async fn post_account_create(
    State(s): State<AppState>,
    Json(body): Json<AccountCreateBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();

    let entry = LedgerEntry::AccountCreate {
        account: body.account,
        keys: body.keys.unwrap_or_default(),
        epoch,
    };

    apply_and_broadcast(&s, entry, None)
}

/// POST /api/account/update-key
/// Body: { "account", "role", "new_public_key", "signed_by", "signature" }
#[derive(Debug, Deserialize)]
struct AccountUpdateKeyBody {
    account: String,
    role: String,
    new_public_key: String,
    signed_by: String,
    #[serde(default)]
    signature: String,
}

async fn post_account_update_key(
    State(s): State<AppState>,
    Json(body): Json<AccountUpdateKeyBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();
    let entry = LedgerEntry::AccountUpdateKey {
        account: body.account,
        role: body.role,
        new_public_key: body.new_public_key,
        epoch,
        signed_by: body.signed_by,
    };
    let sig = non_empty(&body.signature);
    apply_and_broadcast(&s, entry, sig)
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Return `Some(s)` if `s` is non-empty, else `None`.
fn non_empty(s: &str) -> Option<&str> {
    if s.is_empty() { None } else { Some(s) }
}

/// Validate, apply, broadcast, and return the standard JSON response.
fn apply_and_broadcast(
    s: &AppState,
    entry: LedgerEntry,
    sig_hex: Option<&str>,
) -> Json<serde_json::Value> {
    match tx::validate_and_apply(&s.chain, &entry, sig_hex) {
        Ok(hash) => {
            // Carry the signature alongside the entry so gossip peers can re-verify.
            let _ = s.tx_broadcast.send((entry, sig_hex.map(str::to_owned)));
            Json(serde_json::json!({
                "hash": hash,
                "accepted": true,
                "error": null,
            }))
        }
        Err(e) => Json(serde_json::json!({
            "hash": null,
            "accepted": false,
            "error": e.to_string(),
        })),
    }
}

// ── Contract endpoints ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ContractDeployBody {
    deployer: String,
    wasm_b64: String,
    init_method: Option<String>,
    init_args: Option<serde_json::Value>,
    #[serde(default = "default_gas")]
    gas: u64,
    nonce: u64,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct ContractCallBody {
    contract_id: String,
    method: String,
    #[serde(default)]
    args: serde_json::Value,
    signer: String,
    #[serde(deserialize_with = "deserialize_amount_dreams", default)]
    deposit: u64,
    #[serde(default = "default_gas")]
    gas: u64,
    nonce: u64,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct ContractViewBody {
    contract_id: String,
    method: String,
    #[serde(default)]
    args: serde_json::Value,
    #[serde(default = "default_gas")]
    gas: u64,
}

fn default_gas() -> u64 { 300_000_000_000 }

/// POST /api/contract/deploy
async fn post_contract_deploy(
    State(s): State<AppState>,
    Json(body): Json<ContractDeployBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();

    // Verify deployer owns this request.
    let msg = serde_json::json!({
        "type": "CONTRACT_DEPLOY",
        "deployer": &body.deployer,
        "nonce": body.nonce,
        "epoch": epoch,
    });
    if let Ok(msg_bytes) = serde_json::to_vec(&msg) {
        if let Err(e) = tx::check_sig_raw(&s.chain, &body.deployer, &msg_bytes, non_empty(&body.signature)) {
            return Json(serde_json::json!({ "contract_id": null, "ok": false, "error": e.to_string() }));
        }
    }

    match s.contracts.deploy(&body.deployer, &body.wasm_b64, body.init_method, body.init_args, body.gas, epoch, body.nonce) {
        Ok(contract_id) => Json(serde_json::json!({ "contract_id": contract_id, "ok": true, "error": null })),
        Err(e) => Json(serde_json::json!({ "contract_id": null, "ok": false, "error": e.to_string() })),
    }
}

/// POST /api/contract/call
async fn post_contract_call(
    State(s): State<AppState>,
    Json(body): Json<ContractCallBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();

    // Verify signer identity before execution.
    let msg = serde_json::json!({
        "type": "CONTRACT_CALL",
        "signer": &body.signer,
        "contract_id": &body.contract_id,
        "method": &body.method,
        "nonce": body.nonce,
        "epoch": epoch,
    });
    if let Ok(msg_bytes) = serde_json::to_vec(&msg) {
        if let Err(e) = tx::check_sig_raw(&s.chain, &body.signer, &msg_bytes, non_empty(&body.signature)) {
            return Json(serde_json::json!({ "result": null, "ok": false, "error": e.to_string() }));
        }
    }

    match s.contracts.call(&body.contract_id, &body.method, body.args, &body.signer, body.gas, body.deposit, epoch, body.nonce) {
        Ok(result) => Json(serde_json::json!({ "result": result, "ok": true, "error": null })),
        Err(e) => Json(serde_json::json!({ "result": null, "ok": false, "error": e.to_string() })),
    }
}

/// POST /api/contract/view
async fn post_contract_view(
    State(s): State<AppState>,
    Json(body): Json<ContractViewBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();
    match s.contracts.view(&body.contract_id, &body.method, body.args, body.gas, epoch) {
        Ok(result) => Json(serde_json::json!({ "result": result, "ok": true, "error": null })),
        Err(e) => Json(serde_json::json!({ "result": null, "ok": false, "error": e.to_string() })),
    }
}

// ── Inference request bodies ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct InferencePostBody {
    requester: String,
    model: String,
    #[serde(default = "default_mode")]
    mode: String,
    input_hash: String,
    #[serde(deserialize_with = "deserialize_amount_dreams")]
    max_fee: u64,
    #[serde(default)]
    min_reputation: u64,
    #[serde(default = "default_bid_window")]
    bid_window_epochs: u64,
    deadline_epoch: u64,
    nonce: u64,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct InferenceBidBody {
    job_id: String,
    bidder: String,
    #[serde(deserialize_with = "deserialize_amount_dreams")]
    fee: u64,
    #[serde(default = "default_worker_role")]
    role: String,
    nonce: u64,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct InferenceCompleteBody {
    job_id: String,
    worker: String,
    result_hash: String,
    latency_ms: u64,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct InferenceVerifyBody {
    job_id: String,
    verifier: String,
    verdict: String,
    reason: Option<String>,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct InferenceClaimBody {
    job_id: String,
    claimant: String,
    evidence_hash: Option<String>,
    nonce: u64,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct InferenceReviewBody {
    job_id: String,
    reviewer: String,
    approved: bool,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct InferenceCancelBody {
    job_id: String,
    cancelled_by: String,
    #[serde(default)]
    reason: String,
    nonce: u64,
    #[serde(default)]
    signature: String,
}

fn default_mode() -> String { "solo".to_owned() }
fn default_bid_window() -> u64 { 2 }
fn default_worker_role() -> String { "worker".to_owned() }

// ── Inference handlers ────────────────────────────────────────────────────────

async fn post_inference_job(
    State(s): State<AppState>,
    Json(body): Json<InferencePostBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();
    let job_id = {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(body.requester.as_bytes());
        h.update(body.nonce.to_le_bytes());
        h.update(epoch.to_le_bytes());
        hex::encode(h.finalize())[..16].to_owned()
    };
    let requester = body.requester.clone();
    let entry = LedgerEntry::InferenceJobPost {
        job_id: job_id.clone(),
        requester: body.requester,
        model: body.model,
        mode: body.mode,
        input_hash: body.input_hash,
        max_fee: body.max_fee,
        min_reputation: body.min_reputation,
        bid_window_epochs: body.bid_window_epochs,
        deadline_epoch: body.deadline_epoch,
        epoch,
        nonce: body.nonce,
        signed_by: requester,
    };
    let sig = non_empty(&body.signature);
    match tx::validate_and_apply(&s.chain, &entry, sig) {
        Ok(hash) => {
            let _ = s.tx_broadcast.send((entry, sig.map(str::to_owned)));
            Json(serde_json::json!({ "hash": hash, "job_id": job_id, "accepted": true, "error": null }))
        }
        Err(e) => Json(serde_json::json!({ "hash": null, "job_id": null, "accepted": false, "error": e.to_string() })),
    }
}

async fn post_inference_bid(
    State(s): State<AppState>,
    Json(body): Json<InferenceBidBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();
    let entry = LedgerEntry::InferenceJobBid {
        job_id: body.job_id,
        bidder: body.bidder.clone(),
        fee: body.fee,
        role: body.role,
        epoch,
        nonce: body.nonce,
        signed_by: body.bidder,
    };
    let sig = non_empty(&body.signature);
    apply_and_broadcast(&s, entry, sig)
}

async fn post_inference_complete(
    State(s): State<AppState>,
    Json(body): Json<InferenceCompleteBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();
    let entry = LedgerEntry::InferenceJobComplete {
        job_id: body.job_id,
        worker: body.worker.clone(),
        result_hash: body.result_hash,
        latency_ms: body.latency_ms,
        epoch,
        signed_by: body.worker,
    };
    let sig = non_empty(&body.signature);
    apply_and_broadcast(&s, entry, sig)
}

async fn post_inference_verify(
    State(s): State<AppState>,
    Json(body): Json<InferenceVerifyBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();
    let entry = LedgerEntry::InferenceJobVerify {
        job_id: body.job_id,
        verifier: body.verifier.clone(),
        verdict: body.verdict,
        reason: body.reason,
        epoch,
        signed_by: body.verifier,
    };
    let sig = non_empty(&body.signature);
    apply_and_broadcast(&s, entry, sig)
}

async fn post_inference_claim(
    State(s): State<AppState>,
    Json(body): Json<InferenceClaimBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();
    let entry = LedgerEntry::InferenceJobClaim {
        job_id: body.job_id,
        claimant: body.claimant.clone(),
        evidence_hash: body.evidence_hash,
        epoch,
        nonce: body.nonce,
        signed_by: body.claimant,
    };
    let sig = non_empty(&body.signature);
    apply_and_broadcast(&s, entry, sig)
}

async fn post_inference_review(
    State(s): State<AppState>,
    Json(body): Json<InferenceReviewBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();
    let entry = LedgerEntry::InferenceReviewVote {
        job_id: body.job_id,
        reviewer: body.reviewer.clone(),
        approved: body.approved,
        epoch,
        signed_by: body.reviewer,
    };
    let sig = non_empty(&body.signature);
    apply_and_broadcast(&s, entry, sig)
}

async fn post_inference_cancel(
    State(s): State<AppState>,
    Json(body): Json<InferenceCancelBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();
    let entry = LedgerEntry::InferenceJobCancel {
        job_id: body.job_id,
        cancelled_by: body.cancelled_by.clone(),
        reason: body.reason,
        epoch,
        nonce: body.nonce,
        signed_by: body.cancelled_by,
    };
    let sig = non_empty(&body.signature);
    apply_and_broadcast(&s, entry, sig)
}

// GET /api/inference/jobs?status=posted&model=llama3
async fn get_inference_jobs(
    State(s): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let status_filter = params.get("status").map(String::as_str);
    let model_filter = params.get("model").map(String::as_str);

    let jobs: Vec<serde_json::Value> = s.chain.store.state_scan_prefix("infer_job:")
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<inference::JobState>(&v).ok())
        .filter(|j| {
            status_filter.map_or(true, |sf| format!("{:?}", j.status).to_lowercase() == sf)
                && model_filter.map_or(true, |m| j.model == m)
        })
        .filter_map(|j| serde_json::to_value(&j).ok())
        .collect();

    let count = jobs.len();
    Json(serde_json::json!({ "jobs": jobs, "count": count }))
}

// GET /api/inference/job/:id
async fn get_inference_job(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match inference::get_job(&s.chain, &id) {
        Some(job) => {
            let bids = inference::get_bids(&s.chain, &id);
            let votes = inference::get_votes(&s.chain, &id);
            Ok(Json(serde_json::json!({
                "job": job,
                "bids": bids,
                "review_votes": votes,
            })))
        }
        None => Err(StatusCode::NOT_FOUND),
    }
}

// GET /api/inference/reputation/:node
async fn get_inference_reputation(
    State(s): State<AppState>,
    Path(node): Path<String>,
) -> Json<serde_json::Value> {
    let rep = inference::get_reputation(&s.chain, &node);
    Json(serde_json::to_value(&rep).unwrap_or_default())
}

// ── Faucet (testnet only) ─────────────────────────────────────────────────────

const FAUCET_AMOUNT_DREAMS: u64 = 10 * 10_000_000_000; // 10 BTCPC
const FAUCET_COOLDOWN_SECS: u64 = 3600; // 1 hour

#[derive(Debug, Deserialize)]
struct FaucetClaimBody {
    account: String,
}

/// POST /api/faucet/claim
/// Body: { "account": "..." }
/// Returns 403 on mainnet.  Rate-limited to 1 claim per account per hour on testnet.
async fn post_faucet_claim(
    State(s): State<AppState>,
    Json(body): Json<FaucetClaimBody>,
) -> (StatusCode, Json<serde_json::Value>) {
    if s.chain.chain_id != TESTNET_CHAIN_ID {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({
            "success": false,
            "message": "Faucet not available on mainnet. Acquire BTCPC by mining or transfers.",
        })));
    }

    if body.account.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, Json(serde_json::json!({
            "success": false,
            "message": "account must not be empty",
        })));
    }

    let now = Instant::now();
    {
        let mut claims = s.faucet_claims.lock();
        if let Some(&last) = claims.get(body.account.as_str()) {
            let elapsed = now.duration_since(last).as_secs();
            if elapsed < FAUCET_COOLDOWN_SECS {
                return (StatusCode::TOO_MANY_REQUESTS, Json(serde_json::json!({
                    "success": false,
                    "message": format!("Faucet cooldown: {} seconds remaining", FAUCET_COOLDOWN_SECS - elapsed),
                })));
            }
        }
        claims.insert(body.account.clone(), now);
    }

    let epoch = s.chain.current_epoch();
    // Create account if it doesn't exist yet (idempotent).
    let _ = s.chain.apply_entry(&LedgerEntry::AccountCreate {
        account: body.account.clone(),
        keys: Default::default(),
        epoch,
    });

    match s.chain.store.credit(&body.account, NATIVE_TOKEN, FAUCET_AMOUNT_DREAMS) {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({
            "success": true,
            "message": format!("Sent 10 BTCPC to {}", body.account),
            "amount": 10.0_f64,
            "dreams": FAUCET_AMOUNT_DREAMS,
        }))),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "success": false,
            "message": e.to_string(),
        }))),
    }
}

pub async fn serve(state: AppState, port: u16) -> anyhow::Result<()> {
    let app = router(state);
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("API listening on http://{}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

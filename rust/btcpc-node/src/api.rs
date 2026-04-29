//! HTTP API server (Axum) — replaces Node.js btcpc-api.

use std::sync::Arc;
use axum::{
    Router,
    routing::{get, post},
    extract::{Path, State},
    Json, http::StatusCode,
};
use serde::Deserialize;
use tower_http::cors::CorsLayer;
use tokio::sync::broadcast;
use btcpc_types::{Block, LedgerEntry, NATIVE_TOKEN, DREAMS_PER_BTCPC};

use crate::chain::Chain;
use crate::contracts::ContractEngine;
use crate::tx;

#[derive(Clone)]
pub struct AppState {
    pub chain: Arc<Chain>,
    pub contracts: Arc<ContractEngine>,
    /// Broadcast channel for gossiping newly accepted entries to the net module.
    pub tx_broadcast: broadcast::Sender<LedgerEntry>,
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
        .route("/api/entry", post(post_entry))
        // ── Contract endpoints ────────────────────────────────────────────
        .route("/api/contract/deploy", post(post_contract_deploy))
        .route("/api/contract/call", post(post_contract_call))
        .route("/api/contract/view", post(post_contract_view))
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
    let map: serde_json::Map<String, serde_json::Value> = balances.into_iter()
        .map(|(token, d)| (token, serde_json::json!(d as f64 / DREAMS_PER_BTCPC as f64)))
        .collect();
    Json(serde_json::json!({ "account": account, "balances": map }))
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
    Path(epoch): Path<u32>,
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
                "has_block": s.chain.store.has_block(epoch as u32),
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

// ── POST request bodies ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TransferBody {
    from: String,
    to: String,
    /// Amount in BTCPC (fractional OK; will be converted to Dreams).
    amount: f64,
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
    /// Amount in BTCPC.
    amount: f64,
    signed_by: String,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct UnstakeBody {
    account: String,
    /// Amount in BTCPC.
    amount: f64,
    signed_by: String,
    #[serde(default)]
    signature: String,
}

#[derive(Debug, Deserialize)]
struct AccountCreateBody {
    account: String,
    /// Hex-encoded ed25519 public key (optional for watch-only accounts).
    #[serde(default)]
    public_key: Option<String>,
}

fn default_token() -> String {
    NATIVE_TOKEN.to_owned()
}

// ── POST handlers ─────────────────────────────────────────────────────────────

/// POST /api/transfer
/// Body: { "from", "to", "amount" (BTCPC), "token", "memo", "signed_by", "nonce", "signature" }
async fn post_transfer(
    State(s): State<AppState>,
    Json(body): Json<TransferBody>,
) -> Json<serde_json::Value> {
    let amount_dreams = btcpc_to_dreams(body.amount);
    let epoch = s.chain.current_epoch();

    let entry = LedgerEntry::Transfer {
        from: body.from,
        to: body.to,
        amount: amount_dreams,
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
/// Body: { "account", "amount" (BTCPC), "signed_by", "signature" }
async fn post_stake(
    State(s): State<AppState>,
    Json(body): Json<StakeBody>,
) -> Json<serde_json::Value> {
    let amount_dreams = btcpc_to_dreams(body.amount);
    let epoch = s.chain.current_epoch();

    let entry = LedgerEntry::Stake {
        account: body.account,
        amount: amount_dreams,
        epoch,
        signed_by: body.signed_by,
    };

    let sig = non_empty(&body.signature);
    apply_and_broadcast(&s, entry, sig)
}

/// POST /api/unstake
/// Body: { "account", "amount" (BTCPC), "signed_by", "signature" }
async fn post_unstake(
    State(s): State<AppState>,
    Json(body): Json<UnstakeBody>,
) -> Json<serde_json::Value> {
    let amount_dreams = btcpc_to_dreams(body.amount);
    let epoch = s.chain.current_epoch();

    let entry = LedgerEntry::Unstake {
        account: body.account,
        amount: amount_dreams,
        epoch,
        signed_by: body.signed_by,
    };

    let sig = non_empty(&body.signature);
    apply_and_broadcast(&s, entry, sig)
}

/// POST /api/account/create
/// Body: { "account", "public_key" (hex, optional) }
async fn post_account_create(
    State(s): State<AppState>,
    Json(body): Json<AccountCreateBody>,
) -> Json<serde_json::Value> {
    let epoch = s.chain.current_epoch();

    let entry = LedgerEntry::AccountCreate {
        account: body.account,
        public_key: body.public_key,
        epoch,
    };

    apply_and_broadcast(&s, entry, None)
}

/// POST /api/entry — raw LedgerEntry submission for advanced / node-to-node use.
/// Body: any LedgerEntry JSON (must include `"type"` discriminant).
async fn post_entry(
    State(s): State<AppState>,
    Json(raw): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    // Extract an optional top-level "signature" field before parsing the entry
    // so callers can attach it out-of-band.
    let external_sig = raw
        .get("_signature")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    match tx::entry_from_json(&raw) {
        Err(e) => Json(serde_json::json!({
            "hash": null,
            "accepted": false,
            "error": format!("parse error: {}", e),
        })),
        Ok(entry) => apply_and_broadcast(&s, entry, external_sig.as_deref()),
    }
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// Convert BTCPC (f64) → Dreams (u64), rounding to nearest integer.
fn btcpc_to_dreams(btcpc: f64) -> u64 {
    (btcpc * DREAMS_PER_BTCPC as f64).round() as u64
}

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
            // Best-effort broadcast — ignore if no receivers are connected yet.
            let _ = s.tx_broadcast.send(entry);
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
}

#[derive(Debug, Deserialize)]
struct ContractCallBody {
    contract_id: String,
    method: String,
    #[serde(default)]
    args: serde_json::Value,
    signer: String,
    #[serde(default)]
    deposit: f64,
    #[serde(default = "default_gas")]
    gas: u64,
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
    match s.contracts.deploy(&body.deployer, &body.wasm_b64, body.init_method, body.init_args, body.gas, epoch) {
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
    let deposit_dreams = (body.deposit * DREAMS_PER_BTCPC as f64).round() as u64;
    match s.contracts.call(&body.contract_id, &body.method, body.args, &body.signer, body.gas, deposit_dreams, epoch) {
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

pub async fn serve(state: AppState, port: u16) -> anyhow::Result<()> {
    let app = router(state);
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("API listening on http://{}", addr);
    axum::serve(listener, app).await?;
    Ok(())
}

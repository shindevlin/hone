use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::{
    app::AppState,
    auth::AuthUser,
    bonding_curve::{capacity_for_payment, cost_for_capacity, stake_for_capacity},
    ledger,
    models::{current_epoch, CapacityQuoteRequest, LedgerEntry, LinkShippingRequest, OpenStoreRequest, PaginationQuery, UpdateStoreRequest},
};

/// POST /api/commerce/stores — open a storefront
pub async fn open_store(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<OpenStoreRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let seller = user.username;
    {
        let state = app.state.read();
        if let Some(store) = state.stores.get(&seller) {
            if store.status == "active" {
                return Err((StatusCode::CONFLICT, Json(json!({"error": "store already open"}))));
            }
        }
    }

    let name = body.name
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| seller.clone());

    let capacity = body.initial_capacity.unwrap_or(10).max(1).min(10_000);
    let stake_paid_usd = body.stake_paid_usd.unwrap_or(0.0).max(0.0);
    let stake_amount = stake_for_capacity(capacity);
    let epoch = current_epoch();

    let mut entry = LedgerEntry::new("STORE_OPEN", &seller, epoch);
    entry.store_data = Some(json!({
        "action": "open",
        "name": name,
        "banner_cid": body.banner_cid,
        "description_cid": body.description_cid,
        "categories": body.categories.unwrap_or_default(),
        "capacity": capacity,
        "stake_amount": stake_amount,
        "stake_paid_usd": stake_paid_usd,
    }));

    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({
        "ok": true,
        "seller": seller,
        "name": name,
        "capacity": capacity,
        "stake_amount": stake_amount,
        "epoch": epoch,
    })))
}

/// GET /api/commerce/stores — list all active stores
pub async fn list_stores(
    State(app): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Json<Value> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);
    let state = app.state.read();
    let mut stores: Vec<&crate::models::Store> = state.stores.values()
        .filter(|s| s.status == "active")
        .collect();
    stores.sort_by(|a, b| b.opened_at.cmp(&a.opened_at));
    let total = stores.len();
    let page: Vec<Value> = stores.into_iter().skip(offset).take(limit)
        .map(|s| serde_json::to_value(s).unwrap_or(json!({})))
        .collect();
    Json(json!({ "stores": page, "total": total, "limit": limit, "offset": offset }))
}

/// GET /api/commerce/stores/:seller — get a single store
pub async fn get_store(
    State(app): State<AppState>,
    Path(seller): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let state = app.state.read();
    state.stores.get(&seller)
        .map(|s| Json(serde_json::to_value(s).unwrap_or(json!({}))))
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "store not found"}))))
}

/// PATCH /api/commerce/stores/:seller — update store metadata
pub async fn update_store(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(seller): Path<String>,
    Json(body): Json<UpdateStoreRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if user.username != seller {
        return Err((StatusCode::FORBIDDEN, Json(json!({"error": "not your store"}))));
    }
    {
        let state = app.state.read();
        match state.stores.get(&seller) {
            Some(s) if s.status == "active" => {}
            _ => return Err((StatusCode::NOT_FOUND, Json(json!({"error": "store not found or closed"})))),
        }
    }
    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("STORE_UPDATE", &seller, epoch);
    entry.store_data = Some(json!({
        "action": "update",
        "name": body.name,
        "banner_cid": body.banner_cid,
        "description_cid": body.description_cid,
        "categories": body.categories,
    }));
    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    Ok(Json(json!({ "ok": true })))
}

/// DELETE /api/commerce/stores/:seller — close store
pub async fn close_store(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(seller): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if user.username != seller {
        return Err((StatusCode::FORBIDDEN, Json(json!({"error": "not your store"}))));
    }
    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("STORE_CLOSE", &seller, epoch);
    entry.store_data = Some(json!({ "action": "close" }));
    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    Ok(Json(json!({ "ok": true, "seller": seller })))
}

/// POST /api/commerce/stores/:seller/tor/setup — configure Tor hidden service for this store
pub async fn tor_setup(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(seller): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if user.username != seller {
        return Err((StatusCode::FORBIDDEN, Json(json!({"error": "not your store"}))));
    }

    let base     = std::path::Path::new(&app.cfg.data_dir);
    let tor_dir  = base.join("market-tor");
    let hs_dir   = tor_dir.join("hs");
    let torrc    = tor_dir.join("torrc");
    let hostname = hs_dir.join("hostname");

    tokio::fs::create_dir_all(&hs_dir).await
        .map_err(|e: std::io::Error| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    // Tor requires strict permissions on the HS directory (700)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&hs_dir) {
            let mut perms = meta.permissions();
            perms.set_mode(0o700);
            let _ = std::fs::set_permissions(&hs_dir, perms);
        }
    }

    let torrc_content = format!(
        "# HONE Market hidden service\nHiddenServiceDir {}\nHiddenServicePort 80 127.0.0.1:{}\n",
        hs_dir.display(), app.cfg.port
    );
    tokio::fs::write(&torrc, &torrc_content).await
        .map_err(|e: std::io::Error| { let _ = e; })
        .ok();

    // If Tor has already generated the hostname, register it on-chain
    if let Ok(raw) = tokio::fs::read_to_string(&hostname).await {
        let onion: String = raw.trim().to_string();
        if !onion.is_empty() {
            let epoch = current_epoch();
            let mut entry = LedgerEntry::new("STORE_UPDATE", &seller, epoch);
            entry.store_data = Some(json!({
                "action": "update",
                "onion_address": onion,
                "tor_enabled": true,
            }));
            ledger::persist(&app.cfg, &app.state, &entry).ok();

            return Ok(Json(json!({
                "ok": true,
                "status": "active",
                "onion_address": onion,
                "registered_on_chain": true,
            })));
        }
    }

    // Tor not running yet — return config for vendor to apply
    Ok(Json(json!({
        "ok": false,
        "status": "pending_setup",
        "torrc_path": torrc.display().to_string(),
        "torrc_content": torrc_content,
        "hs_dir": hs_dir.display().to_string(),
        "hostname_path": hostname.display().to_string(),
        "next_steps": [
            "Install Tor if needed: sudo apt install tor",
            format!("Run: tor -f {} --RunAsDaemon 1", torrc.display()),
            "Wait ~30 seconds for Tor to generate your .onion address",
            "Then call POST /api/commerce/stores/{seller}/tor/setup again to register it on-chain",
        ],
    })))
}

/// DELETE /api/commerce/stores/:seller/tor — disable Tor routing for this store
pub async fn tor_disable(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(seller): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if user.username != seller {
        return Err((StatusCode::FORBIDDEN, Json(json!({"error": "not your store"}))));
    }
    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("STORE_UPDATE", &seller, epoch);
    entry.store_data = Some(json!({ "action": "update", "tor_enabled": false }));
    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    Ok(Json(json!({ "ok": true, "tor_enabled": false })))
}

const VALID_CARRIERS: &[&str] = &["ups", "fedex", "usps", "dhl", "pirateship", "shipstation", "easypost", "other"];

/// POST /api/commerce/stores/:seller/shipping — link a carrier account
pub async fn link_shipping(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(seller): Path<String>,
    Json(body): Json<LinkShippingRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if user.username != seller {
        return Err((StatusCode::FORBIDDEN, Json(json!({"error": "not your store"}))));
    }
    {
        let state = app.state.read();
        match state.stores.get(&seller) {
            Some(s) if s.status == "active" => {}
            _ => return Err((StatusCode::NOT_FOUND, Json(json!({"error": "store not found or closed"})))),
        }
    }
    let carrier = body.carrier.to_lowercase();
    if !VALID_CARRIERS.contains(&carrier.as_str()) {
        return Err((StatusCode::BAD_REQUEST, Json(json!({
            "error": "unsupported carrier",
            "supported": VALID_CARRIERS,
        }))));
    }
    if body.account_id.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "account_id required"}))));
    }
    let masked = if body.account_id.len() <= 4 {
        "*".repeat(body.account_id.len())
    } else {
        format!("****{}", &body.account_id[body.account_id.len() - 4..])
    };
    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("STORE_SHIPPING_LINK", &seller, epoch);
    entry.store_data = Some(json!({
        "carrier": carrier,
        "account_id": body.account_id.trim(),
        "default_service": body.default_service.unwrap_or_else(|| "ground".to_string()),
    }));
    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    Ok(Json(json!({ "ok": true, "carrier": carrier, "account_id_masked": masked })))
}

/// DELETE /api/commerce/stores/:seller/shipping/:carrier — unlink a carrier
pub async fn unlink_shipping(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((seller, carrier)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if user.username != seller {
        return Err((StatusCode::FORBIDDEN, Json(json!({"error": "not your store"}))));
    }
    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("STORE_SHIPPING_UNLINK", &seller, epoch);
    entry.store_data = Some(json!({ "carrier": carrier.to_lowercase() }));
    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    Ok(Json(json!({ "ok": true, "carrier": carrier.to_lowercase() })))
}

/// GET /api/commerce/stores/:seller/metrics — public store performance metrics
pub async fn get_store_metrics(
    State(app): State<AppState>,
    Path(seller): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let state = app.state.read();
    if !state.stores.contains_key(&seller) {
        return Err((StatusCode::NOT_FOUND, Json(json!({"error": "store not found"}))));
    }

    let seller_orders: Vec<&crate::models::Order> = state.orders.values()
        .filter(|o| o.seller == seller)
        .collect();

    let total_orders = seller_orders.len() as u64;
    let fulfilled_orders = seller_orders.iter().filter(|o| o.status == "fulfilled" || o.status == "delivered").count() as u64;
    let disputed_orders = seller_orders.iter().filter(|o| o.status == "disputed").count() as u64;
    let cancelled_orders = seller_orders.iter().filter(|o| o.status == "cancelled").count() as u64;

    // Fulfilled orders that have a recorded fulfilled_epoch
    let fulfilled_orders_vec: Vec<&&crate::models::Order> = seller_orders.iter()
        .filter(|o| (o.status == "fulfilled" || o.status == "delivered") && o.fulfilled_epoch.is_some())
        .collect();

    // Late fulfillments: fulfilled AFTER the deadline
    let late_fulfillments = fulfilled_orders_vec.iter()
        .filter(|o| o.fulfill_deadline_epoch > 0 && o.fulfilled_epoch.unwrap_or(0) > o.fulfill_deadline_epoch)
        .count() as u64;

    // Average fulfillment time in epochs (fulfilled_epoch - placed_epoch)
    let avg_fulfill_epochs: f64 = if fulfilled_orders_vec.is_empty() {
        0.0
    } else {
        let sum: u64 = fulfilled_orders_vec.iter()
            .filter_map(|o| o.fulfilled_epoch.map(|fe| fe.saturating_sub(o.placed_epoch)))
            .sum();
        sum as f64 / fulfilled_orders_vec.len() as f64
    };

    // Repeat buyers: buyers with >1 delivered order from this seller
    let mut buyer_counts: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    for o in seller_orders.iter().filter(|o| o.status == "delivered") {
        *buyer_counts.entry(o.buyer.as_str()).or_insert(0) += 1;
    }
    let repeat_buyer_count = buyer_counts.values().filter(|&&c| c > 1).count() as u64;

    Ok(Json(json!({
        "seller": seller,
        "total_orders": total_orders,
        "fulfilled_orders": fulfilled_orders,
        "disputed_orders": disputed_orders,
        "cancelled_orders": cancelled_orders,
        "late_fulfillments": late_fulfillments,
        "avg_fulfill_epochs": avg_fulfill_epochs,
        "repeat_buyer_count": repeat_buyer_count,
    })))
}

/// GET /api/commerce/quote/capacity — bonding curve price quote
pub async fn quote_capacity(
    Query(q): Query<CapacityQuoteRequest>,
) -> Json<Value> {
    let current = q.current_capacity.unwrap_or(0);
    let units = q.units.max(1).min(100_000);
    let cost_usd = cost_for_capacity(current, units);
    let slots_for_1 = capacity_for_payment(current, 1.0);
    let stake_needed = stake_for_capacity(units);
    Json(json!({
        "current_capacity": current,
        "units": units,
        "cost_usd": cost_usd,
        "slots_for_1_usd": slots_for_1,
        "stake_hone_required": stake_needed,
    }))
}

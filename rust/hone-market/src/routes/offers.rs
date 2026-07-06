use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{
    app::AppState,
    auth::AuthUser,
    ledger,
    models::{current_epoch, CounterOfferRequest, LedgerEntry, PlaceOfferRequest},
};

fn gen_offer_id() -> String {
    let id = Uuid::new_v4().to_string().replace('-', "");
    format!("ofr-{}", &id[..16])
}

fn gen_order_id() -> String {
    let id = Uuid::new_v4().to_string().replace('-', "");
    format!("ord-{}", &id[..24])
}

fn gen_escrow_id() -> String {
    let id = Uuid::new_v4().to_string().replace('-', "");
    format!("esc-{}", &id[..16])
}

/// POST /api/commerce/offers — place a new offer
pub async fn place_offer(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<PlaceOfferRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let buyer = user.username;

    if body.amount < 0.0 {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "amount must be >= 0"}))));
    }

    let seller = {
        let state = app.state.read();
        let product = state.products.get(&body.product_id)
            .filter(|p| p.status == "active")
            .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "product not found or unlisted"}))))?;
        if product.seller == buyer {
            return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "cannot make offer on your own product"}))));
        }
        product.seller.clone()
    };

    let epoch = current_epoch();
    let offer_id = gen_offer_id();
    let expires_epoch = epoch + 2880;

    let mut entry = LedgerEntry::new("OFFER_PLACE", &buyer, epoch);
    entry.to = Some(seller.clone());
    entry.order_data = Some(json!({
        "offer_id": offer_id,
        "product_id": body.product_id,
        "amount": body.amount,
        "message": body.message,
        "expires_epoch": expires_epoch,
    }));

    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    let offer = {
        let state = app.state.read();
        state.offers.get(&offer_id).map(|o| serde_json::to_value(o).unwrap_or(json!({})))
    };

    Ok(Json(json!({
        "ok": true,
        "offer": offer,
    })))
}

/// GET /api/commerce/offers/my — list caller's offers (as buyer or seller)
pub async fn my_offers(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Json<Value> {
    let username = user.username;
    let state = app.state.read();
    let offers: Vec<Value> = state.offers.values()
        .filter(|o| o.buyer == username || o.seller == username)
        .map(|o| serde_json::to_value(o).unwrap_or(json!({})))
        .collect();
    let total = offers.len();
    Json(json!({ "offers": offers, "total": total }))
}

/// POST /api/commerce/offers/:oid/counter — seller counters an offer
pub async fn counter_offer(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(offer_id): Path<String>,
    Json(body): Json<CounterOfferRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let seller = user.username;

    {
        let state = app.state.read();
        let offer = state.offers.get(&offer_id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "offer not found"}))))?;
        if offer.seller != seller {
            return Err((StatusCode::FORBIDDEN, Json(json!({"error": "not your offer to counter"}))));
        }
        if offer.status != "pending" {
            return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error": "offer is not in pending state"}))));
        }
    }

    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("OFFER_COUNTER", &seller, epoch);
    entry.order_data = Some(json!({
        "offer_id": offer_id,
        "counter_amount": body.amount,
        "counter_message": body.message,
    }));

    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({ "ok": true, "offer_id": offer_id })))
}

/// POST /api/commerce/offers/:oid/accept — accept an offer (buyer or seller)
pub async fn accept_offer(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(offer_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let username = user.username;

    let (offer_amount, offer_product_id, offer_buyer, offer_seller, offer_token) = {
        let state = app.state.read();
        let offer = state.offers.get(&offer_id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "offer not found"}))))?;

        // Seller accepting buyer's original offer — status must be "pending"
        // Buyer accepting seller's counter — status must be "countered"
        if offer.seller == username {
            if offer.status != "pending" {
                return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error": "offer is not pending"}))));
            }
        } else if offer.buyer == username {
            if offer.status != "countered" {
                return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error": "no counter offer to accept"}))));
            }
        } else {
            return Err((StatusCode::FORBIDDEN, Json(json!({"error": "not your offer"}))));
        }

        let accept_amount = if offer.seller == username {
            offer.amount
        } else {
            offer.counter_amount.unwrap_or(offer.amount)
        };

        let token = state.products.get(&offer.product_id)
            .map(|p| p.token.clone())
            .unwrap_or_else(|| "HONE".to_string());

        (accept_amount, offer.product_id.clone(), offer.buyer.clone(), offer.seller.clone(), token)
    };

    let epoch = current_epoch();
    let order_id = gen_order_id();
    let escrow_id = gen_escrow_id();

    // Write OFFER_ACCEPT
    let mut accept_entry = LedgerEntry::new("OFFER_ACCEPT", &username, epoch);
    accept_entry.order_data = Some(json!({
        "offer_id": offer_id,
        "order_id": order_id,
    }));
    ledger::persist(&app.cfg, &app.state, &accept_entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    // Write ORDER_PLACE using the offer amount
    let total = (offer_amount * 1e10).round() / 1e10;
    let mut order_entry = LedgerEntry::new("ORDER_PLACE", &offer_buyer, epoch);
    order_entry.to = Some(offer_seller.clone());
    order_entry.token = offer_token.clone();
    order_entry.amount = total;
    order_entry.order_data = Some(json!({
        "order_id": order_id,
        "product_id": offer_product_id,
        "quantity": 1,
        "unit_price": offer_amount,
        "total": total,
        "token": offer_token,
        "escrow_id": escrow_id,
        "shipping_address": null,
        "offer_id": offer_id,
    }));
    ledger::persist(&app.cfg, &app.state, &order_entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({
        "ok": true,
        "offer_id": offer_id,
        "order_id": order_id,
        "amount": offer_amount,
        "escrow_id": escrow_id,
    })))
}

/// POST /api/commerce/offers/:oid/reject — reject an offer
pub async fn reject_offer(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(offer_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let username = user.username;

    {
        let state = app.state.read();
        let offer = state.offers.get(&offer_id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "offer not found"}))))?;
        if offer.buyer != username && offer.seller != username {
            return Err((StatusCode::FORBIDDEN, Json(json!({"error": "not your offer"}))));
        }
        if !matches!(offer.status.as_str(), "pending" | "countered") {
            return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error": "offer cannot be rejected in current state"}))));
        }
    }

    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("OFFER_REJECT", &username, epoch);
    entry.order_data = Some(json!({ "offer_id": offer_id }));

    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({ "ok": true, "offer_id": offer_id, "status": "rejected" })))
}

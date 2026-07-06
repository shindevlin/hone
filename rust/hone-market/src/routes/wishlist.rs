use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::{
    app::AppState,
    auth::AuthUser,
    ledger,
    models::{current_epoch, LedgerEntry},
};

/// POST /api/commerce/wishlist/:pid — add product to wishlist
pub async fn add_wishlist(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(product_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let buyer = user.username;

    {
        let state = app.state.read();
        state.products.get(&product_id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "product not found"}))))?;
    }

    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("WISHLIST_ADD", &buyer, epoch);
    entry.product_data = Some(json!({ "product_id": product_id }));

    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({ "ok": true, "product_id": product_id })))
}

/// DELETE /api/commerce/wishlist/:pid — remove product from wishlist
pub async fn remove_wishlist(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(product_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let buyer = user.username;

    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("WISHLIST_REMOVE", &buyer, epoch);
    entry.product_data = Some(json!({ "product_id": product_id }));

    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({ "ok": true, "product_id": product_id })))
}

/// GET /api/commerce/wishlist — get caller's wishlist (full product objects)
pub async fn get_wishlist(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
) -> Json<Value> {
    let username = user.username;
    let state = app.state.read();
    let product_ids = state.wishlists.get(&username).cloned().unwrap_or_default();
    let products: Vec<Value> = product_ids.iter()
        .filter_map(|pid| state.products.get(pid))
        .map(|p| serde_json::to_value(p).unwrap_or(json!({})))
        .collect();
    let total = products.len();
    Json(json!({ "products": products, "total": total }))
}

/// GET /api/commerce/wishlist/count/:pid — public count of wishlists containing this product
pub async fn wishlist_count(
    State(app): State<AppState>,
    Path(product_id): Path<String>,
) -> Json<Value> {
    let state = app.state.read();
    let count = state.wishlists.values()
        .filter(|set| set.contains(&product_id))
        .count();
    Json(json!({ "product_id": product_id, "count": count }))
}

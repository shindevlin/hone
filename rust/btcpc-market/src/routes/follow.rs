use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};

use crate::{
    app::AppState,
    auth::AuthUser,
    ledger,
    models::{current_epoch, LedgerEntry, PaginationQuery},
};

/// POST /api/commerce/stores/:seller/follow — follow a store
pub async fn follow_store(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(seller): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let follower = user.username;

    {
        let state = app.state.read();
        state.stores.get(&seller)
            .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "store not found"}))))?;
    }

    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("FOLLOW_STORE", &follower, epoch);
    entry.store_data = Some(json!({ "seller": seller }));

    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({ "ok": true, "seller": seller })))
}

/// DELETE /api/commerce/stores/:seller/follow — unfollow a store
pub async fn unfollow_store(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(seller): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let follower = user.username;

    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("UNFOLLOW_STORE", &follower, epoch);
    entry.store_data = Some(json!({ "seller": seller }));

    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({ "ok": true, "seller": seller })))
}

/// GET /api/commerce/feed — get products from followed stores
pub async fn get_feed(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Query(q): Query<PaginationQuery>,
) -> Json<Value> {
    let username = user.username;
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);

    let state = app.state.read();

    // Collect sellers this user follows
    let followed_sellers: std::collections::HashSet<String> = state.store_followers.iter()
        .filter(|(_, followers)| followers.contains(&username))
        .map(|(seller, _)| seller.clone())
        .collect();

    let mut products: Vec<&crate::models::Product> = state.products.values()
        .filter(|p| p.status == "active" && followed_sellers.contains(&p.seller))
        .collect();

    products.sort_by(|a, b| b.created_epoch.cmp(&a.created_epoch));
    let total = products.len();
    let page: Vec<Value> = products.into_iter().skip(offset).take(limit)
        .map(|p| serde_json::to_value(p).unwrap_or(json!({})))
        .collect();

    Json(json!({ "products": page, "total": total, "limit": limit, "offset": offset }))
}

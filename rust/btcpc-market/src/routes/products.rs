use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    Json,
};
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};

use crate::{
    app::AppState,
    auth::AuthUser,
    ledger,
    models::{current_epoch, BumpProductRequest, CreateProductRequest, LedgerEntry, PaginationQuery, UpdateProductRequest},
};

static SLUG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-z0-9][a-z0-9\-]{0,62}$").unwrap());

fn valid_slug(s: &str) -> bool {
    SLUG_RE.is_match(s)
}

fn build_product_id(seller: &str, slug: &str) -> String {
    format!("{}/{}", seller, slug)
}

/// POST /api/commerce/products — list a new product
pub async fn create_product(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Json(body): Json<CreateProductRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let seller = user.username;

    // Must have an active store
    {
        let state = app.state.read();
        let store = state.stores.get(&seller)
            .filter(|s| s.status == "active")
            .ok_or_else(|| (StatusCode::BAD_REQUEST, Json(json!({"error": "open a store first"}))))?;

        if store.used_capacity >= store.capacity {
            return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error": "store capacity full"}))));
        }
    }

    if !valid_slug(&body.slug) {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "invalid slug — lowercase alphanumeric and dashes only"}))));
    }
    if body.price < 0.0 {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "price must be >= 0"}))));
    }
    if body.title.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"error": "title required"}))));
    }

    let product_id = build_product_id(&seller, &body.slug);
    {
        let state = app.state.read();
        if state.products.get(&product_id).map(|p| p.status == "active").unwrap_or(false) {
            return Err((StatusCode::CONFLICT, Json(json!({"error": "product_id already exists"}))));
        }
    }

    let epoch = current_epoch();
    let token = body.token.unwrap_or_else(|| "BTCPC".to_string());

    let mut entry = LedgerEntry::new("PRODUCT_CREATE", &seller, epoch);
    entry.product_data = Some(json!({
        "product_id": product_id,
        "title": body.title,
        "description": body.description,
        "price": body.price,
        "token": token,
        "image_cid": body.image_cid,
        "inventory": body.inventory,
        "categories": body.categories.unwrap_or_default(),
        "auto_deliver": body.auto_deliver.unwrap_or(false),
        "delivery_cid": body.delivery_cid,
        "sale_price": body.sale_price,
        "sale_ends_epoch": body.sale_ends_epoch,
        "min_price": body.min_price,
        "suggested_price": body.suggested_price,
        "tip_enabled": body.tip_enabled.unwrap_or(false),
        "condition": body.condition,
        "condition_notes": body.condition_notes,
        "expected_delivery_epoch": body.expected_delivery_epoch,
        "specs": body.specs,
        "image_cids": body.image_cids.unwrap_or_default(),
    }));

    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;

    Ok(Json(json!({
        "ok": true,
        "product_id": product_id,
        "seller": seller,
        "epoch": epoch,
    })))
}

/// GET /api/commerce/products — list products
pub async fn list_products(
    State(app): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> Json<Value> {
    let limit = q.limit.unwrap_or(50).min(200);
    let offset = q.offset.unwrap_or(0);
    let search = q.q.as_deref().unwrap_or("").to_lowercase();
    let cat_filter = q.category.as_deref().unwrap_or("").to_lowercase();
    let state = app.state.read();
    let mut products: Vec<&crate::models::Product> = state.products.values()
        .filter(|p| p.status == "active")
        .filter(|p| q.seller.as_ref().map(|s| &p.seller == s).unwrap_or(true))
        .filter(|p| {
            if search.is_empty() { return true; }
            p.title.to_lowercase().contains(&search)
                || p.description.as_deref().unwrap_or("").to_lowercase().contains(&search)
                || p.seller.to_lowercase().contains(&search)
        })
        .filter(|p| {
            if cat_filter.is_empty() { return true; }
            p.categories.iter().any(|c| c.to_lowercase().contains(&cat_filter))
        })
        .collect();
    let epoch = current_epoch();
    let sort_key = q.sort.as_deref().unwrap_or("bumped_first");
    products.sort_by(|a, b| {
        match sort_key {
            "price_asc" => a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal),
            "price_desc" => b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal),
            "newest" => b.created_epoch.cmp(&a.created_epoch),
            _ => {
                // default: bumped first, then newest
                let a_bumped = a.bumped_until_epoch.map_or(false, |e| e > epoch);
                let b_bumped = b.bumped_until_epoch.map_or(false, |e| e > epoch);
                b_bumped.cmp(&a_bumped).then(b.created_epoch.cmp(&a.created_epoch))
            }
        }
    });
    let total = products.len();
    let page: Vec<Value> = products.into_iter().skip(offset).take(limit)
        .map(|p| serde_json::to_value(p).unwrap_or(json!({})))
        .collect();
    Json(json!({ "products": page, "total": total, "limit": limit, "offset": offset }))
}

/// GET /api/commerce/products/:productId — get product by id (seller/slug)
pub async fn get_product(
    State(app): State<AppState>,
    Path(seller_slug): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let state = app.state.read();
    state.products.get(&seller_slug)
        .map(|p| Json(serde_json::to_value(p).unwrap_or(json!({}))))
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "product not found"}))))
}

/// PATCH /api/commerce/products/:productId — update product
pub async fn update_product(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(product_id): Path<String>,
    Json(body): Json<UpdateProductRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let seller = user.username;
    {
        let state = app.state.read();
        let product = state.products.get(&product_id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "product not found"}))))?;
        if product.seller != seller {
            return Err((StatusCode::FORBIDDEN, Json(json!({"error": "not your product"}))));
        }
    }

    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("PRODUCT_UPDATE", &seller, epoch);
    entry.product_data = Some(json!({
        "product_id": product_id,
        "title": body.title,
        "description": body.description,
        "price": body.price,
        "image_cid": body.image_cid,
        "inventory": body.inventory,
        "auto_deliver": body.auto_deliver,
        "delivery_cid": body.delivery_cid,
        "sale_price": body.sale_price,
        "sale_ends_epoch": body.sale_ends_epoch,
        "min_price": body.min_price,
        "suggested_price": body.suggested_price,
        "tip_enabled": body.tip_enabled,
        "condition": body.condition,
        "condition_notes": body.condition_notes,
        "expected_delivery_epoch": body.expected_delivery_epoch,
        "specs": body.specs,
        "image_cids": body.image_cids,
        "bumped_until_epoch": body.bumped_until_epoch,
    }));

    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    Ok(Json(json!({ "ok": true })))
}

/// POST /api/commerce/products/*pid/bump — bump a listing
pub async fn bump_product(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(product_id): Path<String>,
    Json(body): Json<BumpProductRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let seller = user.username;
    {
        let state = app.state.read();
        let product = state.products.get(&product_id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "product not found"}))))?;
        if product.seller != seller {
            return Err((StatusCode::FORBIDDEN, Json(json!({"error": "not your product"}))));
        }
        if product.status != "active" {
            return Err((StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error": "product not active"}))));
        }
    }
    let epochs = body.epochs.max(120).min(2880);
    let epoch = current_epoch();
    let bumped_until_epoch = epoch + epochs;

    let mut entry = LedgerEntry::new("PRODUCT_BUMP", &seller, epoch);
    entry.product_data = Some(json!({
        "product_id": product_id,
        "bumped_until_epoch": bumped_until_epoch,
    }));

    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    Ok(Json(json!({ "ok": true, "product_id": product_id, "bumped_until_epoch": bumped_until_epoch })))
}

/// GET /api/commerce/categories — list all categories with product count
pub async fn list_categories(
    State(app): State<AppState>,
) -> Json<Value> {
    let state = app.state.read();
    let mut counts: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for product in state.products.values().filter(|p| p.status == "active") {
        for cat in &product.categories {
            *counts.entry(cat.clone()).or_insert(0) += 1;
        }
    }
    let mut categories: Vec<serde_json::Value> = counts.into_iter()
        .map(|(cat, count)| json!({ "category": cat, "count": count }))
        .collect();
    categories.sort_by(|a, b| {
        b["count"].as_u64().unwrap_or(0).cmp(&a["count"].as_u64().unwrap_or(0))
    });
    Json(json!({ "categories": categories }))
}

/// DELETE /api/commerce/products/:productId — delist product
pub async fn delist_product(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path(product_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let seller = user.username;
    {
        let state = app.state.read();
        let product = state.products.get(&product_id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "product not found"}))))?;
        if product.seller != seller {
            return Err((StatusCode::FORBIDDEN, Json(json!({"error": "not your product"}))));
        }
    }

    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("PRODUCT_DELIST", &seller, epoch);
    entry.product_data = Some(json!({ "product_id": product_id }));

    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    Ok(Json(json!({ "ok": true, "product_id": product_id })))
}

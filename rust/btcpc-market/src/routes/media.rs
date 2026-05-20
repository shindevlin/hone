use axum::{
    extract::{Extension, Multipart, State},
    http::StatusCode,
    Json,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

use crate::{app::AppState, auth::AuthUser};

const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024; // 8 MB

/// POST /api/commerce/media/upload
/// Accepts a single image file (multipart/form-data, field name "file").
/// Stores the blob in $BTCPC_DATA_DIR/market-blobs/<sha256hex>
/// and returns a BTCPC-FS CID: "sha256:<hex>".
pub async fn upload_image(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    mut multipart: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let blobs_dir = Path::new(&app.cfg.data_dir).join("market-blobs");
    tokio::fs::create_dir_all(&blobs_dir).await.map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})))
    })?;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()})))
    })? {
        let ct = field.content_type()
            .unwrap_or("application/octet-stream")
            .to_string();

        if !ct.starts_with("image/") {
            return Err((StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": "only image/* files accepted"}))));
        }

        let data = field.bytes().await.map_err(|e| {
            (StatusCode::BAD_REQUEST, Json(json!({"error": e.to_string()})))
        })?;

        if data.len() > MAX_IMAGE_BYTES {
            return Err((StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({"error": "image must be under 8 MB"}))));
        }

        let mut hasher = Sha256::new();
        hasher.update(&data);
        let hash = format!("{:x}", hasher.finalize());
        let cid  = format!("sha256:{hash}");

        let file_path = blobs_dir.join(&hash);
        if !file_path.exists() {
            tokio::fs::write(&file_path, &data).await.map_err(|e| {
                (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()})))
            })?;
        }

        // Serve the uploaded image at /api/commerce/media/:hash
        return Ok(Json(json!({
            "ok":           true,
            "cid":          cid,
            "hash":         hash,
            "size":         data.len(),
            "content_type": ct,
            "uploader":     user.username,
            "url":          format!("/api/commerce/media/{hash}"),
        })));
    }

    Err((StatusCode::BAD_REQUEST, Json(json!({"error": "no file in request"}))))
}

/// GET /api/commerce/media/:hash — serve an uploaded product image
pub async fn serve_image(
    State(app): State<AppState>,
    axum::extract::Path(hash): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    use axum::http::{header, HeaderMap};

    if hash.contains('/') || hash.contains('.') {
        return Err(StatusCode::BAD_REQUEST);
    }
    let path = Path::new(&app.cfg.data_dir).join("market-blobs").join(&hash);
    match tokio::fs::read(&path).await {
        Ok(data) => {
            let mut headers = HeaderMap::new();
            headers.insert(header::CONTENT_TYPE, "image/jpeg".parse().unwrap());
            headers.insert(header::CACHE_CONTROL, "public, max-age=31536000, immutable".parse().unwrap());
            Ok((headers, data))
        }
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

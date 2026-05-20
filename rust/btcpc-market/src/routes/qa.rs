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
    models::{AskQuestionRequest, AnswerQuestionRequest, LedgerEntry, current_epoch},
};

fn gen_qa_id() -> String {
    let id = Uuid::new_v4().to_string().replace('-', "");
    format!("qa-{}", &id[..16])
}

/// POST /api/commerce/products/:seller/:slug/qa — ask a question
pub async fn ask_question(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((seller, slug)): Path<(String, String)>,
    Json(body): Json<AskQuestionRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let product_id = format!("{}/{}", seller, slug);
    {
        let state = app.state.read();
        state.products.get(&product_id)
            .filter(|p| p.status == "active")
            .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "product not found"}))))?;
    }
    let qa_id = gen_qa_id();
    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("PRODUCT_QA_ASK", &user.username, epoch);
    entry.product_data = Some(json!({
        "product_id": product_id,
        "qa_id": qa_id,
        "question": body.question.trim(),
    }));
    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    Ok(Json(json!({ "ok": true, "qa_id": qa_id })))
}

/// PATCH /api/commerce/products/:seller/:slug/qa/:qa_id — answer a question (seller only)
pub async fn answer_question(
    State(app): State<AppState>,
    Extension(user): Extension<AuthUser>,
    Path((seller, slug, qa_id)): Path<(String, String, String)>,
    Json(body): Json<AnswerQuestionRequest>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let product_id = format!("{}/{}", seller, slug);
    {
        let state = app.state.read();
        let product = state.products.get(&product_id)
            .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "product not found"}))))?;
        if product.seller != user.username {
            return Err((StatusCode::FORBIDDEN, Json(json!({"error": "only seller can answer"}))));
        }
    }
    let epoch = current_epoch();
    let mut entry = LedgerEntry::new("PRODUCT_QA_ANSWER", &user.username, epoch);
    entry.product_data = Some(json!({
        "product_id": product_id,
        "qa_id": qa_id,
        "answer": body.answer.trim(),
    }));
    ledger::persist(&app.cfg, &app.state, &entry)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))))?;
    Ok(Json(json!({ "ok": true })))
}

/// GET /api/commerce/products/:seller/:slug/qa — list questions
pub async fn list_questions(
    State(app): State<AppState>,
    Path((seller, slug)): Path<(String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let product_id = format!("{}/{}", seller, slug);
    let state = app.state.read();
    let product = state.products.get(&product_id)
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"error": "product not found"}))))?;
    let questions: Vec<Value> = product.questions.iter()
        .map(|q| serde_json::to_value(q).unwrap_or(json!({})))
        .collect();
    let total = questions.len();
    Ok(Json(json!({ "questions": questions, "total": total })))
}

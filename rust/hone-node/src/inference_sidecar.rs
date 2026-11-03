//! `hone-node inference-serve` — the inference sidecar.
//!
//! The node is the front door for inference (API, routing, pricing, fees, verification,
//! payments and rewards all stay in the node and in consensus) but never runs a model in
//! its own process. The model runs here, in a separate process with its own systemd unit
//! and memory budget, and the node reaches it over localhost through the existing
//! INFERENCE_URL path. The sidecar can crash, restart or OOM without touching the node:
//! the node's calls fail with an error (bounded by HONE_INFERENCE_TIMEOUT_SECS) and
//! succeed again once it is back.
//!
//! Ollama-compatible surface (what `inference_engine::http_chat` and /api/node/models use):
//!   POST /api/chat     {model, messages:[{role,content}], stream:false, options:{num_predict}}
//!                      -> {model, message:{role:"assistant",content}, done:true}
//!   GET  /api/tags     -> {models:[{name}]}
//!   GET  /v1/models    -> {data:[{id}]}
//!   GET  /health       -> {ok, model, available}
//!
//! Env: HONE_INFERENCE_LISTEN (default 127.0.0.1:4250 — loopback only), HONE_MODEL_DIR,
//! HONE_MODEL (the GGUF to serve), HONE_INFERENCE_MAX_TOKENS (cap, default 512).
//! The sidecar holds no keys, opens no chain store and joins no network.

use anyhow::Result;
use axum::{routing::{get, post}, Json, Router};
use serde::Deserialize;

use crate::inference_engine::{self, ChatRequest, Message};

#[derive(Deserialize)]
struct ChatBody {
    #[serde(default)]
    model: String,
    messages: Vec<Message>,
    #[serde(default)]
    options: Option<serde_json::Value>,
}

fn model_name() -> String { std::env::var("HONE_MODEL").unwrap_or_default() }

fn max_tokens_cap() -> usize {
    std::env::var("HONE_INFERENCE_MAX_TOKENS").ok().and_then(|v| v.parse().ok()).unwrap_or(512)
}

async fn chat(Json(b): Json<ChatBody>) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    let want = b.options.as_ref().and_then(|o| o["num_predict"].as_u64()).unwrap_or(256) as usize;
    let req = ChatRequest { model: b.model.clone(), messages: b.messages, max_tokens: want.min(max_tokens_cap()).max(1) };
    match inference_engine::chat(req).await {
        Ok(r) => (axum::http::StatusCode::OK, Json(serde_json::json!({
            "model": if b.model.is_empty() { model_name() } else { b.model },
            "message": { "role": "assistant", "content": r.content },
            "done": true,
        }))),
        Err(e) => (axum::http::StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({ "error": e.to_string() }))),
    }
}

pub fn router() -> Router {
    Router::new()
        .route("/api/chat", post(chat))
        .route("/api/tags", get(|| async { Json(serde_json::json!({ "models": [{ "name": model_name() }] })) }))
        .route("/v1/models", get(|| async { Json(serde_json::json!({ "data": [{ "id": model_name() }] })) }))
        .route("/health", get(|| async { Json(serde_json::json!({
            "ok": true, "model": model_name(), "backend": inference_engine::backend_name(),
            "available": inference_engine::available(),
        })) }))
}

pub async fn serve() -> Result<()> {
    inference_engine::set_sidecar();
    let listen = std::env::var("HONE_INFERENCE_LISTEN").unwrap_or_else(|_| "127.0.0.1:4250".into());
    tracing::info!("hone-inference: backend {} model '{}' on {}", inference_engine::backend_name(), model_name(), listen);
    inference_engine::warm_up().await;
    let l = tokio::net::TcpListener::bind(&listen).await?;
    axum::serve(l, router()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The node's HTTP backend talks to the sidecar's shape: a node pointed at a running
    /// sidecar gets the answer, and a dead sidecar is an error, not a hang or a crash.
    #[tokio::test]
    async fn node_http_backend_round_trips_through_a_sidecar_and_survives_its_death() {
        // A stand-in sidecar with the same routes (the real one needs a GGUF on disk).
        let app = Router::new().route("/api/chat", post(|Json(b): Json<serde_json::Value>| async move {
            let last = b["messages"].as_array().and_then(|m| m.last()).and_then(|m| m["content"].as_str()).unwrap_or("").to_owned();
            Json(serde_json::json!({ "message": { "role": "assistant", "content": format!("echo:{last}") }, "done": true }))
        }));
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let srv = tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        let base = format!("http://{addr}");
        let req = || ChatRequest { model: "m".into(), messages: vec![Message { role: "user".into(), content: "hi".into() }], max_tokens: 8 };
        assert_eq!(inference_engine::http_chat_for_test(&base, &req()).await.unwrap().content, "echo:hi");
        srv.abort();
        let _ = srv.await;
        assert!(inference_engine::http_chat_for_test(&base, &req()).await.is_err(), "sidecar down = error");
    }

    #[test]
    fn empty_or_auto_model_uses_the_configured_default() {
        assert_eq!(inference_engine::resolve_model_for_test("qwen3:4b"), "qwen3:4b");
        std::env::set_var("HONE_INFERENCE_MODEL", "llama3.2:1b");
        assert_eq!(inference_engine::resolve_model_for_test(""), "llama3.2:1b");
        assert_eq!(inference_engine::resolve_model_for_test("auto"), "llama3.2:1b");
    }

    #[tokio::test]
    async fn sidecar_routes_answer_without_a_model() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, router()).await.unwrap() });
        let h: serde_json::Value = reqwest::get(format!("http://{addr}/health")).await.unwrap().json().await.unwrap();
        assert_eq!(h["ok"], true);
        let r = reqwest::Client::new().post(format!("http://{addr}/api/chat"))
            .json(&serde_json::json!({ "messages": [{ "role": "user", "content": "x" }] }))
            .send().await.unwrap();
        assert_eq!(r.status(), 503, "no model -> 503, never a crash");
    }
}

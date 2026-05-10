//! Model healer background task.
//!
//! Keeps a healthy Ollama model available by running a 7-step fallback chain
//! with exponential backoff. Writes healer events to CF_META for /api/node/info.

use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::chain::Chain;

const FALLBACK_CHAIN: &[&str] = &[
    // steps 3-7 (preferred model is injected at runtime as steps 0-2)
    "qwen3:4b",
    "llama3.2:1b",
    "qwen2.5:0.5b",
];
const BACKOFF_SECS: &[u64] = &[1, 2, 5, 10, 30, 60];

pub async fn run(chain: Arc<Chain>, ollama_url: String, preferred_model: String) {
    info!("[model-healer] started — preferred={}", preferred_model);

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    let mut backoff_idx = 0usize;

    loop {
        let model = pick_healthy_model(&http, &ollama_url, &preferred_model).await;
        match model {
            Some(m) => {
                if m != preferred_model {
                    warn!("[model-healer] preferred unavailable, using fallback: {}", m);
                }
                record_event(&chain, "healthy", &m);
                backoff_idx = 0;
                tokio::time::sleep(Duration::from_secs(300)).await;
            }
            None => {
                let secs = BACKOFF_SECS[backoff_idx.min(BACKOFF_SECS.len() - 1)];
                warn!("[model-healer] no model available — retrying in {}s", secs);
                record_event(&chain, "no_model", "");
                backoff_idx = (backoff_idx + 1).min(BACKOFF_SECS.len() - 1);
                tokio::time::sleep(Duration::from_secs(secs)).await;
            }
        }
    }
}

async fn pick_healthy_model(
    http: &reqwest::Client,
    ollama_url: &str,
    preferred: &str,
) -> Option<String> {
    // Step 0: check if preferred responds.
    if ping_model(http, ollama_url, preferred).await {
        return Some(preferred.to_owned());
    }

    // Step 1: try to pull preferred.
    if pull_model(http, ollama_url, preferred).await {
        return Some(preferred.to_owned());
    }

    // Step 2: list locally available models, pick largest.
    if let Some(local) = largest_local_model(http, ollama_url).await {
        if ping_model(http, ollama_url, &local).await {
            return Some(local);
        }
    }

    // Steps 3-7: pull fallback chain in order.
    for &fallback in FALLBACK_CHAIN {
        if pull_model(http, ollama_url, fallback).await {
            return Some(fallback.to_owned());
        }
    }

    None
}

async fn ping_model(http: &reqwest::Client, ollama_url: &str, model: &str) -> bool {
    let url = format!("{}/api/generate", ollama_url);
    let body = serde_json::json!({
        "model": model,
        "prompt": "ping",
        "stream": false,
        "options": { "num_predict": 1 }
    });
    http.post(&url)
        .json(&body)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn pull_model(http: &reqwest::Client, ollama_url: &str, model: &str) -> bool {
    info!("[model-healer] pulling {}", model);
    let url = format!("{}/api/pull", ollama_url);
    let body = serde_json::json!({ "name": model, "stream": false });
    // Pull can take minutes for large models; generous timeout.
    http.post(&url)
        .json(&body)
        .timeout(Duration::from_secs(600))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

async fn largest_local_model(http: &reqwest::Client, ollama_url: &str) -> Option<String> {
    let url = format!("{}/api/tags", ollama_url);
    let resp = http.get(&url)
        .timeout(Duration::from_secs(10))
        .send().await.ok()?
        .json::<serde_json::Value>().await.ok()?;

    let models = resp["models"].as_array()?;
    models.iter()
        .filter_map(|m| {
            let name = m["name"].as_str()?.to_owned();
            let size = m["size"].as_u64().unwrap_or(0);
            Some((name, size))
        })
        .max_by_key(|(_, size)| *size)
        .map(|(name, _)| name)
}

fn record_event(chain: &Arc<Chain>, event: &str, model: &str) {
    let val = serde_json::json!({
        "event": event,
        "model": model,
        "ts": crate::utils::now_ms(),
    });
    if let Ok(bytes) = serde_json::to_vec(&val) {
        let _ = chain.store.state_set("healer:last_event", &bytes);
    }
}

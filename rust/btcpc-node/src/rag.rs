//! RAG service — Ollama-based embedding, cosine ranking, 6k context injection.
//! P5-A: ~550 LOC

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};

use crate::chain::Chain;

const MAX_CONTEXT_CHARS: usize = 6_144; // ~6k characters injected into prompt
const EMBEDDING_MODEL: &str = "nomic-embed-text";
const TOP_K: usize = 5;
const OLLAMA_BASE: &str = "http://127.0.0.1:11434";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RagDocument {
    pub doc_id: String,
    pub account: String,
    pub content: String,
    pub content_hash: String,
    pub embedding: Vec<f32>,
    pub indexed_epoch: u64,
}

fn doc_key(doc_id: &str) -> String { format!("rag_doc:{}", doc_id) }
fn index_key(account: &str) -> String { format!("rag_index:{}", account) }

/// Add or update a document in the RAG index.
pub async fn index_document(
    chain: &Chain,
    account: &str,
    doc_id: &str,
    content: &str,
    epoch: u64,
) -> Result<()> {
    let embedding = embed(content).await?;
    let content_hash = hex::encode(Sha256::digest(content.as_bytes()));
    let doc = RagDocument {
        doc_id: doc_id.to_string(),
        account: account.to_string(),
        content: content.to_string(),
        content_hash,
        embedding,
        indexed_epoch: epoch,
    };
    chain.store.state_set(&doc_key(doc_id), &serde_json::to_vec(&doc)?)?;
    // Add doc_id to account's index
    let mut index: Vec<String> = chain.store.state_get(&index_key(account))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if !index.contains(&doc_id.to_string()) {
        index.push(doc_id.to_string());
        chain.store.state_set(&index_key(account), &serde_json::to_vec(&index)?)?;
    }
    Ok(())
}

/// Delete a document from the index.
pub fn delete_document(chain: &Chain, account: &str, doc_id: &str) -> Result<()> {
    chain.store.state_delete(&doc_key(doc_id))?;
    let mut index: Vec<String> = chain.store.state_get(&index_key(account))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    index.retain(|id| id != doc_id);
    chain.store.state_set(&index_key(account), &serde_json::to_vec(&index)?)?;
    Ok(())
}

/// Query the RAG index: returns top-K documents + a 6k context string.
pub async fn query(
    chain: &Chain,
    account: &str,
    query_text: &str,
) -> Result<RagResult> {
    let query_emb = embed(query_text).await?;
    let index: Vec<String> = chain.store.state_get(&index_key(account))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();

    let mut scored: Vec<(f32, RagDocument)> = index.iter()
        .filter_map(|id| {
            let raw = chain.store.state_get(&doc_key(id))?;
            let doc: RagDocument = serde_json::from_slice(&raw).ok()?;
            let score = cosine_similarity(&query_emb, &doc.embedding);
            Some((score, doc))
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(TOP_K);

    let context = build_context(&scored, MAX_CONTEXT_CHARS);
    let sources: Vec<RagSource> = scored.iter().map(|(score, doc)| RagSource {
        doc_id: doc.doc_id.clone(),
        score: *score,
        excerpt: doc.content.chars().take(200).collect(),
    }).collect();

    Ok(RagResult { context, sources })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RagResult {
    /// Context string ready to inject into LLM prompt (<= 6k chars).
    pub context: String,
    pub sources: Vec<RagSource>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RagSource {
    pub doc_id: String,
    pub score: f32,
    pub excerpt: String,
}

fn build_context(scored: &[(f32, RagDocument)], max_chars: usize) -> String {
    let mut out = String::new();
    for (_, doc) in scored {
        let chunk = format!("[{}]\n{}\n\n", doc.doc_id, doc.content);
        if out.len() + chunk.len() > max_chars { break; }
        out.push_str(&chunk);
    }
    out.truncate(max_chars);
    out
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() { return 0.0; }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 { 0.0 } else { dot / (norm_a * norm_b) }
}

/// Call Ollama /api/embeddings and return the embedding vector.
async fn embed(text: &str) -> Result<Vec<f32>> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "model": EMBEDDING_MODEL,
        "prompt": text,
    });
    let resp: serde_json::Value = client
        .post(format!("{}/api/embeddings", OLLAMA_BASE))
        .json(&body)
        .send()
        .await?
        .json()
        .await?;
    let emb: Vec<f32> = resp["embedding"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Ollama embeddings response missing 'embedding' field"))?
        .iter()
        .filter_map(|v| v.as_f64().map(|f| f as f32))
        .collect();
    anyhow::ensure!(!emb.is_empty(), "received empty embedding from Ollama");
    Ok(emb)
}

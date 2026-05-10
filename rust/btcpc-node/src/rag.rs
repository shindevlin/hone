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

/// Sync variant — record document metadata on-chain without calling Ollama.
/// Stores a metadata-only record (`content_hash`, `chunk_count`) and updates
/// the account's doc-id index.  Used by chain entries and tests.
pub fn apply_index(
    chain: &Chain,
    account: &str,
    doc_id: &str,
    content_hash: &str,
    chunk_count: u64,
    epoch: u64,
) -> Result<()> {
    let meta = serde_json::json!({
        "doc_id": doc_id,
        "account": account,
        "content_hash": content_hash,
        "chunk_count": chunk_count,
        "indexed_epoch": epoch,
    });
    chain.store.state_set(&doc_key(doc_id), &serde_json::to_vec(&meta)?)?;
    let mut index: Vec<String> = chain.store.state_get(&index_key(account))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if !index.contains(&doc_id.to_string()) {
        index.push(doc_id.to_string());
        chain.store.state_set(&index_key(account), &serde_json::to_vec(&index)?)?;
    }
    Ok(())
}

/// Sync delete alias — removes the doc and updates the index.
pub fn apply_delete(chain: &Chain, account: &str, doc_id: &str) -> Result<()> {
    delete_document(chain, account, doc_id)
}

/// Return the list of doc metadata values for the given account.
pub fn get_docs(chain: &Chain, account: &str) -> Vec<serde_json::Value> {
    let index: Vec<String> = chain.store.state_get(&index_key(account))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    index.iter()
        .filter_map(|id| {
            chain.store.state_get(&doc_key(id))
                .and_then(|b| serde_json::from_slice(&b).ok())
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chain(label: &str) -> (Chain, tempfile::TempDir) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("btcpc_test_{}_", label))
            .tempdir()
            .unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(store, format!("node-{}", label), "btcpc-test".to_string());
        (chain, dir)
    }

    #[test]
    fn test_index_stores_doc() {
        let (chain, _dir) = make_chain("rag_index");
        apply_index(&chain, "alice", "doc1", "abc123", 5, 1).unwrap();
        let docs = get_docs(&chain, "alice");
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0]["doc_id"].as_str().unwrap(), "doc1");
        assert_eq!(docs[0]["chunk_count"].as_u64().unwrap(), 5);
    }

    #[test]
    fn test_delete_removes_doc() {
        let (chain, _dir) = make_chain("rag_delete");
        apply_index(&chain, "alice", "doc2", "def456", 3, 1).unwrap();
        apply_delete(&chain, "alice", "doc2").unwrap();
        let docs = get_docs(&chain, "alice");
        assert!(docs.is_empty(), "doc should be removed from index");
    }

    #[test]
    fn test_get_docs_returns_empty_vec_for_unknown_account() {
        let (chain, _dir) = make_chain("rag_empty");
        let docs = get_docs(&chain, "nobody");
        assert!(docs.is_empty());
    }

    #[test]
    fn test_index_then_delete_yields_empty_list() {
        let (chain, _dir) = make_chain("rag_idx_del");
        apply_index(&chain, "bob", "docA", "hash1", 2, 1).unwrap();
        apply_delete(&chain, "bob", "docA").unwrap();
        assert!(get_docs(&chain, "bob").is_empty());
    }
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

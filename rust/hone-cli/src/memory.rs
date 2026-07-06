use anyhow::Result;
use hone_sdk::KeyPair;
use colored::Colorize;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;
use crate::helpers::{next_nonce, node_chain_id, print_resp, resolve_key_file, sign_entry};

// ── Memory set ────────────────────────────────────────────────────────────────

pub fn cmd_memory_set(account: &str, key: &str, value: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;

    let sig = sign_entry(&kp, &chain_id, "MEMORY_SET", json!({
        "account": account, "key": key, "value": value, "nonce": nonce,
    }));

    let resp: Value = api.post("/api/memory/set", &json!({
        "account": account, "key": key, "value": value,
        "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Memory set.".green().bold());
    print_resp(&resp);
    Ok(())
}

// ── Memory get ────────────────────────────────────────────────────────────────

pub fn cmd_memory_get(account: &str, key: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/memory/get/{}/{}", account, key))?;
    print_resp(&resp);
    Ok(())
}

// ── Memory delete ─────────────────────────────────────────────────────────────

pub fn cmd_memory_delete(account: &str, key: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;

    let sig = sign_entry(&kp, &chain_id, "MEMORY_DELETE", json!({
        "account": account, "key": key, "nonce": nonce,
    }));

    let resp: Value = api.post("/api/memory/del", &json!({
        "account": account, "key": key, "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Memory deleted.".green().bold());
    print_resp(&resp);
    Ok(())
}

// ── Memory scan ───────────────────────────────────────────────────────────────

pub fn cmd_memory_scan(account: &str, prefix: &str) -> Result<()> {
    let api = ApiClient::new();
    let path = format!("/api/memory/scan/{}?prefix={}", account, prefix);
    let resp: Value = api.get(&path)?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

// ── RAG index ─────────────────────────────────────────────────────────────────

pub fn cmd_rag_index(account: &str, doc_id: &str, content: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;

    let sig = sign_entry(&kp, &chain_id, "RAG_INDEX", json!({
        "account": account, "doc_id": doc_id, "nonce": nonce,
    }));

    let resp: Value = api.post("/api/rag/index", &json!({
        "account": account, "doc_id": doc_id, "content": content,
        "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Document indexed.".green().bold());
    print_resp(&resp);
    Ok(())
}

// ── RAG query ─────────────────────────────────────────────────────────────────

pub fn cmd_rag_query(account: &str, query: &str) -> Result<()> {
    let api = ApiClient::new();
    let path = format!("/api/rag/query?account={}&q={}", account,
        urlencoding_simple(query));
    let resp: Value = api.get(&path)?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

// ── RAG delete ────────────────────────────────────────────────────────────────

pub fn cmd_rag_delete(account: &str, doc_id: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;

    let sig = sign_entry(&kp, &chain_id, "RAG_DELETE", json!({
        "account": account, "doc_id": doc_id, "nonce": nonce,
    }));

    let resp: Value = api.post(&format!("/api/rag/doc/{}", doc_id), &json!({
        "account": account, "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Document deleted.".green().bold());
    print_resp(&resp);
    Ok(())
}

fn urlencoding_simple(s: &str) -> String {
    s.chars().map(|c| match c {
        ' ' => '+'.to_string(),
        c if c.is_alphanumeric() || "-_.~".contains(c) => c.to_string(),
        c => format!("%{:02X}", c as u8),
    }).collect()
}

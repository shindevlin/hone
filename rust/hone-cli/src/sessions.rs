use anyhow::Result;
use hone_sdk::KeyPair;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;
use crate::helpers::{next_nonce, node_chain_id, print_resp, resolve_key_file, sign_entry};

// ── Session Market ────────────────────────────────────────────────────────────

pub fn cmd_session_list_create(
    provider: &str, context_summary: &str, price_per_turn: u64,
    max_turns: u32, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, provider)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "SessionListingCreate", json!({
        "provider": provider, "context_summary": context_summary,
        "price_per_turn": price_per_turn, "max_turns": max_turns, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/sessions/list", &json!({
        "provider": provider, "context_summary": context_summary,
        "price_per_turn": price_per_turn, "max_turns": max_turns,
        "nonce": nonce, "signed_by": provider, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_session_buy(
    listing_id: &str, buyer: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, buyer)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "SessionListingBuy", json!({
        "listing_id": listing_id, "buyer": buyer, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/sessions/buy", &json!({
        "listing_id": listing_id, "buyer": buyer,
        "nonce": nonce, "signed_by": buyer, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_session_cancel(
    listing_id: &str, provider: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, provider)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "SessionCancel", json!({
        "listing_id": listing_id, "provider": provider, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/sessions/cancel", &json!({
        "listing_id": listing_id, "provider": provider,
        "nonce": nonce, "signed_by": provider, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_session_listings() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/sessions/listings")?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_session_listing_get(listing_id: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/sessions/listing/{}", listing_id))?;
    print_resp(&resp);
    Ok(())
}

// ── Agent Sessions ────────────────────────────────────────────────────────────

pub fn cmd_agent_session_open(
    requester: &str, agent: &str, max_turns: u32,
    tools: Vec<String>, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, requester)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentSessionOpen", json!({
        "requester": requester, "agent": agent, "max_turns": max_turns,
        "tools": tools, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/agent-session/open", &json!({
        "requester": requester, "agent": agent, "max_turns": max_turns,
        "tools": tools, "nonce": nonce, "signed_by": requester, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_session_close(
    session_id: &str, account: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentSessionClose", json!({
        "session_id": session_id, "account": account, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/agent-session/close", &json!({
        "session_id": session_id, "account": account,
        "nonce": nonce, "signed_by": account, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_session_get(session_id: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/agent-session/{}", session_id))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_session_turn(
    session_id: &str, sender: &str, message: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, sender)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentSessionTurn", json!({
        "session_id": session_id, "sender": sender, "message": message, "nonce": nonce,
    }));
    let resp: Value = api.post(&format!("/api/agent-session/{}/turn", session_id), &json!({
        "sender": sender, "message": message,
        "nonce": nonce, "signed_by": sender, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

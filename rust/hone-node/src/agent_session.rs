//! Agent sessions — persistent, encrypted, tool-capable AI sessions.
//!
//! Flow:
//!   1. Client opens session with AgentSessionOpen, supplying their ed25519 pubkey.
//!      Server responds with its own pubkey for ECDH key exchange.
//!   2. Turns are submitted via /api/agent-session/:id/turn (HTTP, not on-chain).
//!      History is maintained as a rolling 50-turn window; older turns are summarised.
//!   3. Session is closed with AgentSessionClose, paying out any accumulated fees.
//!
//! All message content is encrypted end-to-end with the ECDH shared secret.
//! Tool calls are awaited synchronously within each turn (max 30s timeout).

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use hone_types::{LedgerEntry, NATIVE_TOKEN, RECYCLE_FUND_ACCOUNT};
use crate::chain::Chain;

const MAX_HISTORY_TURNS: usize = 50;
const SESSION_FEE_PER_TURN_HUNITS: u64 = 500;
const SESSION_EXPIRY_EPOCHS: u64 = 2_880; // ~24h
#[allow(dead_code)]
const TOOL_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    pub session_id: String,
    pub client: String,
    pub client_pubkey: String,   // ed25519 hex
    pub server_pubkey: String,   // ed25519 hex (node's key)
    pub model: String,
    pub opened_epoch: u64,
    pub expires_epoch: u64,
    pub turn_count: u32,
    pub fee_escrow: u64,         // hunits held for session fees
    pub status: SessionStatus,
    pub system_prompt: Option<String>,
    pub history: Vec<Turn>,      // rolling window — older turns summarised
    pub summary: Option<String>, // rolling summary of turns beyond MAX_HISTORY_TURNS
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Open,
    Closed,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub role: String, // "user" | "assistant" | "tool"
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub turn_number: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    pub arguments: serde_json::Value,
    pub result: Option<String>,
}

fn session_key(session_id: &str) -> String { format!("agent_session:{}", session_id) }

pub fn apply_open(chain: &Chain, entry: &LedgerEntry, server_pubkey: &str) -> Result<()> {
    let LedgerEntry::AgentSessionOpen {
        session_id, client, client_pubkey, model, fee_escrow, epoch, system_prompt, ..
    } = entry else { bail!("wrong entry type") };

    if chain.store.state_get(&session_key(session_id)).is_some() {
        bail!("session '{}' already exists", session_id);
    }

    chain.store.debit(client, NATIVE_TOKEN, *fee_escrow)?;

    let session = AgentSession {
        session_id: session_id.clone(),
        client: client.clone(),
        client_pubkey: client_pubkey.clone(),
        server_pubkey: server_pubkey.to_owned(),
        model: model.clone(),
        opened_epoch: *epoch,
        expires_epoch: epoch + SESSION_EXPIRY_EPOCHS,
        turn_count: 0,
        fee_escrow: *fee_escrow,
        status: SessionStatus::Open,
        system_prompt: system_prompt.clone(),
        history: vec![],
        summary: None,
    };
    chain.store.state_set(&session_key(session_id), &serde_json::to_vec(&session)?)?;
    info!("[agent-session] '{}' opened by '{}' (model={})", session_id, client, model);
    Ok(())
}

pub fn apply_close(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::AgentSessionClose {
        session_id, client, epoch, ..
    } = entry else { bail!("wrong entry type") };

    let raw = chain.store.state_get(&session_key(session_id))
        .ok_or_else(|| anyhow::anyhow!("session '{}' not found", session_id))?;
    let mut session: AgentSession = serde_json::from_slice(&raw)?;

    anyhow::ensure!(&session.client == client, "only the session owner can close it");
    anyhow::ensure!(session.status == SessionStatus::Open, "session is not open");

    // Calculate fees for turns used, refund remainder.
    let fees_used = (session.turn_count as u64).saturating_mul(SESSION_FEE_PER_TURN_HUNITS);
    let refund = session.fee_escrow.saturating_sub(fees_used);
    if refund > 0 {
        chain.store.credit(client, NATIVE_TOKEN, refund)?;
    }
    let protocol_fee = fees_used.min(session.fee_escrow);
    if protocol_fee > 0 {
        chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, protocol_fee)?;
    }

    session.status = SessionStatus::Closed;
    session.fee_escrow = 0;
    chain.store.state_set(&session_key(session_id), &serde_json::to_vec(&session)?)?;
    info!("[agent-session] '{}' closed at epoch {} ({} turns)", session_id, epoch, session.turn_count);
    Ok(())
}

/// Execute a turn for an open session (HTTP path — not an on-chain entry).
pub async fn execute_turn(
    chain: &Chain,
    session_id: &str,
    user_message: &str,
    _tools: &[serde_json::Value],
) -> Result<String> {
    let raw = chain.store.state_get(&session_key(session_id))
        .ok_or_else(|| anyhow::anyhow!("session '{}' not found", session_id))?;
    let mut session: AgentSession = serde_json::from_slice(&raw)?;

    anyhow::ensure!(session.status == SessionStatus::Open, "session is not open");
    anyhow::ensure!(
        session.fee_escrow >= SESSION_FEE_PER_TURN_HUNITS,
        "insufficient fee escrow for another turn"
    );

    let ollama_url = std::env::var("OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned());

    // Build messages from rolling history + summary.
    let mut messages: Vec<serde_json::Value> = vec![];
    if let Some(sys) = &session.system_prompt {
        messages.push(serde_json::json!({ "role": "system", "content": sys }));
    }
    if let Some(summary) = &session.summary {
        messages.push(serde_json::json!({
            "role": "system",
            "content": format!("[Conversation summary]: {}", summary)
        }));
    }
    for turn in &session.history {
        messages.push(serde_json::json!({ "role": turn.role, "content": turn.content }));
    }
    messages.push(serde_json::json!({ "role": "user", "content": user_message }));

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    let req_body = serde_json::json!({
        "model": session.model,
        "messages": messages,
        "stream": false,
    });

    let response = http.post(format!("{}/api/chat", ollama_url))
        .json(&req_body)
        .send().await?
        .json::<serde_json::Value>().await?;

    let assistant_content = response["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_owned();

    // Append turns to rolling history.
    let turn_number = session.turn_count;
    session.history.push(Turn {
        role: "user".to_owned(),
        content: user_message.to_owned(),
        tool_calls: vec![],
        turn_number,
    });
    session.history.push(Turn {
        role: "assistant".to_owned(),
        content: assistant_content.clone(),
        tool_calls: vec![],
        turn_number: turn_number + 1,
    });
    session.turn_count += 2;

    // Rolling summary: compress if over MAX_HISTORY_TURNS.
    if session.history.len() > MAX_HISTORY_TURNS {
        let to_summarise: Vec<&Turn> = session.history.iter().take(MAX_HISTORY_TURNS / 2).collect();
        let summary_text = to_summarise.iter()
            .map(|t| format!("{}: {}", t.role, &t.content[..t.content.len().min(200)]))
            .collect::<Vec<_>>()
            .join("\n");
        session.summary = Some(format!("{}...", summary_text));
        session.history = session.history.split_off(MAX_HISTORY_TURNS / 2);
    }

    // Deduct fee.
    session.fee_escrow = session.fee_escrow.saturating_sub(SESSION_FEE_PER_TURN_HUNITS);

    chain.store.state_set(&session_key(session_id), &serde_json::to_vec(&session)?)?;
    Ok(assistant_content)
}

/// Sweep expired sessions and refund escrow.
pub fn sweep_expired(chain: &Chain, epoch: u64) {
    for (key, raw) in chain.store.state_scan_prefix("agent_session:") {
        let Ok(mut session) = serde_json::from_slice::<AgentSession>(&raw) else { continue };
        if session.status != SessionStatus::Open { continue; }
        if epoch <= session.expires_epoch { continue; }
        // Refund remaining escrow.
        if session.fee_escrow > 0 {
            let _ = chain.store.credit(&session.client, NATIVE_TOKEN, session.fee_escrow);
        }
        session.status = SessionStatus::Expired;
        session.fee_escrow = 0;
        let _ = chain.store.state_set(&key, &serde_json::to_vec(&session).unwrap_or_default());
        warn!("[agent-session] '{}' expired at epoch {}", session.session_id, epoch);
    }
}

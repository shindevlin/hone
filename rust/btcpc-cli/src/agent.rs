use anyhow::Result;
use btcpc_sdk::KeyPair;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;
use crate::helpers::{next_nonce, node_chain_id, print_resp, resolve_key_file, sign_entry};

// ── Registry ──────────────────────────────────────────────────────────────────

pub fn cmd_agent_register(
    account: &str, name: &str, description: &str,
    tools: Vec<String>, model: &str, min_fee: u64,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentRegister", json!({
        "account": account, "name": name, "description": description,
        "tools": tools, "model": model, "min_fee": min_fee, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/agent/register", &json!({
        "account": account, "name": name, "description": description,
        "tools": tools, "model": model, "min_fee": min_fee,
        "nonce": nonce, "signed_by": account, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_deregister(account: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentDeregister", json!({
        "account": account, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/agent/deregister", &json!({
        "account": account, "nonce": nonce, "signed_by": account, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_registry_list(tool: Option<String>) -> Result<()> {
    let api = ApiClient::new();
    let path = match &tool {
        Some(t) => format!("/api/agent/registry?tool={}", t),
        None => "/api/agent/registry".to_owned(),
    };
    let resp: Value = api.get(&path)?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_registry_get(account: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/agent/registry/{}", account))?;
    print_resp(&resp);
    Ok(())
}

// ── Credit ────────────────────────────────────────────────────────────────────

pub fn cmd_agent_credit_deposit(account: &str, amount: u64, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentCreditDeposit", json!({
        "account": account, "amount": amount, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/agent/credit/deposit", &json!({
        "account": account, "amount": amount,
        "nonce": nonce, "signed_by": account, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_credit_withdraw(account: &str, amount: u64, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentCreditWithdraw", json!({
        "account": account, "amount": amount, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/agent/credit/withdraw", &json!({
        "account": account, "amount": amount,
        "nonce": nonce, "signed_by": account, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_credit_balance(account: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/agent/credit/{}", account))?;
    print_resp(&resp);
    Ok(())
}

// ── Tasks ─────────────────────────────────────────────────────────────────────

pub fn cmd_agent_task_post(
    task_id: &str, requester: &str, description: &str,
    tools: Vec<String>, max_fee: u64, min_verifiers: u32,
    bid_window_epochs: u64, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, requester)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentTaskPost", json!({
        "task_id": task_id, "requester": requester, "description": description,
        "tools_allowed": tools, "max_fee": max_fee, "min_verifiers": min_verifiers,
        "bid_window_epochs": bid_window_epochs, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/agent/task/post", &json!({
        "task_id": task_id, "requester": requester, "description": description,
        "tools_allowed": tools, "max_fee": max_fee, "min_verifiers": min_verifiers,
        "bid_window_epochs": bid_window_epochs,
        "nonce": nonce, "signed_by": requester, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_task_get(task_id: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/agent/task/{}", task_id))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_tasks_list(
    requester: Option<String>, agent: Option<String>, status: Option<String>,
) -> Result<()> {
    let api = ApiClient::new();
    let mut params = vec![];
    if let Some(r) = &requester { params.push(format!("requester={}", r)); }
    if let Some(a) = &agent { params.push(format!("agent={}", a)); }
    if let Some(s) = &status { params.push(format!("status={}", s)); }
    let qs = if params.is_empty() { String::new() } else { format!("?{}", params.join("&")) };
    let resp: Value = api.get(&format!("/api/agent/tasks{}", qs))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_task_bid(
    task_id: &str, agent: &str, proposed_fee: u64, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, agent)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentTaskBid", json!({
        "task_id": task_id, "agent": agent, "proposed_fee": proposed_fee, "nonce": nonce,
    }));
    let resp: Value = api.post(&format!("/api/agent/task/{}/bid", task_id), &json!({
        "agent": agent, "proposed_fee": proposed_fee,
        "nonce": nonce, "signed_by": agent, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_task_assign(
    task_id: &str, agent: &str, fee: u64, signed_by: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, signed_by)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentTaskAssign", json!({
        "task_id": task_id, "agent": agent, "fee": fee, "nonce": nonce,
    }));
    let resp: Value = api.post(&format!("/api/agent/task/{}/assign", task_id), &json!({
        "agent": agent, "fee": fee,
        "nonce": nonce, "signed_by": signed_by, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_task_submit(
    task_id: &str, agent: &str, result_hash: &str, output_cid: &str,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, agent)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentTaskSubmit", json!({
        "task_id": task_id, "agent": agent,
        "result_hash": result_hash, "output_cid": output_cid, "nonce": nonce,
    }));
    let resp: Value = api.post(&format!("/api/agent/task/{}/submit", task_id), &json!({
        "agent": agent, "result_hash": result_hash, "output_cid": output_cid,
        "nonce": nonce, "signed_by": agent, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_verifier_commit(
    task_id: &str, verifier: &str, commit_hash: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, verifier)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentTaskVerifierCommit", json!({
        "task_id": task_id, "verifier": verifier, "commit_hash": commit_hash, "nonce": nonce,
    }));
    let resp: Value = api.post(&format!("/api/agent/task/{}/verify/commit", task_id), &json!({
        "verifier": verifier, "commit_hash": commit_hash,
        "nonce": nonce, "signed_by": verifier, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_agent_verifier_reveal(
    task_id: &str, verifier: &str, result_hash: &str, salt: &str,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, verifier)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "AgentTaskVerifierReveal", json!({
        "task_id": task_id, "verifier": verifier,
        "result_hash": result_hash, "salt": salt, "nonce": nonce,
    }));
    let resp: Value = api.post(&format!("/api/agent/task/{}/verify/reveal", task_id), &json!({
        "verifier": verifier, "result_hash": result_hash, "salt": salt,
        "nonce": nonce, "signed_by": verifier, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

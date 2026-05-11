//! Agent worker daemon — polls for assigned tasks, executes via Ollama,
//! and handles verifier commit-reveal for tasks this node didn't author.
//!
//! Enabled with: BTCPC_AGENT_WORKER=true
//! Verifier-only mode (no task execution): BTCPC_AGENT_VERIFIER=true
//!
//! Each epoch the daemon:
//!   1. Auto-assigns any Open tasks whose bid window has closed (lowest fee wins).
//!   2. Executes Assigned tasks where this node is the agent.
//!   3. For Submitted tasks, acts as verifier: independently runs the task,
//!      commits result hash, then reveals after a short delay.

use std::sync::Arc;
use std::time::Duration;
use sha2::{Digest, Sha256};
use tracing::{info, warn, error};

use btcpc_types::{LedgerEntry, epoch_duration_ms};
use crate::chain::Chain;
use crate::agent_task::{AgentTask, TaskStatus};
use crate::agent_tools;
use crate::net::NetCmd;

const OLLAMA_TIMEOUT_SECS: u64 = 120;
const VERIFIER_REVEAL_DELAY_EPOCHS: u64 = 1; // reveal one epoch after commit

pub async fn run(
    chain: Arc<Chain>,
    account: String,
    cmd_tx: tokio::sync::mpsc::Sender<NetCmd>,
    signing_key: Arc<Option<ed25519_dalek::SigningKey>>,
) {
    info!("[agent-worker] started for account={}", account);
    let mut last_epoch = 0u64;

    loop {
        let epoch = chain.current_epoch();
        if epoch > last_epoch {
            last_epoch = epoch;
            process_epoch(&chain, &account, &cmd_tx, &signing_key, epoch).await;
        }
        let interval = epoch_duration_ms(epoch) / 2;
        tokio::time::sleep(Duration::from_millis(interval.max(5_000))).await;
    }
}

async fn process_epoch(
    chain: &Chain,
    account: &str,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
    signing_key: &Arc<Option<ed25519_dalek::SigningKey>>,
    epoch: u64,
) {
    auto_assign_tasks(chain, account, cmd_tx, signing_key, epoch);
    execute_assigned_tasks(chain, account, cmd_tx, signing_key, epoch).await;
    verify_submitted_tasks(chain, account, cmd_tx, signing_key, epoch).await;
    reveal_committed_verifications(chain, account, cmd_tx, signing_key, epoch).await;
}

/// Auto-assign open tasks whose bid window has closed — lowest fee wins.
fn auto_assign_tasks(
    chain: &Chain,
    account: &str,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
    signing_key: &Arc<Option<ed25519_dalek::SigningKey>>,
    epoch: u64,
) {
    let tasks = scan_tasks(chain, TaskStatus::Open);
    for task in tasks {
        if epoch <= task.posted_epoch + task.bid_window_epochs { continue; }

        // Collect all bids for this task.
        let prefix = format!("agent_task_bid:{}:", task.task_id);
        let mut best_agent: Option<String> = None;
        let mut best_fee = u64::MAX;

        for (key, raw) in chain.store.state_scan_prefix(&prefix).into_iter() {
            let fee: u64 = serde_json::from_slice(&raw).unwrap_or(u64::MAX);
            // Extract agent from key: agent_task_bid:{task_id}:{agent}
            let agent = key.splitn(3, ':').nth(2).unwrap_or("").to_owned();
            // Tiebreak: lower fee wins; SHA-256(task_id|agent) breaks ties.
            let is_better = fee < best_fee || (fee == best_fee && {
                let hash_new = hex::encode(Sha256::digest(format!("{}:{}", task.task_id, agent).as_bytes()));
                let hash_cur = best_agent.as_ref().map(|a| {
                    hex::encode(Sha256::digest(format!("{}:{}", task.task_id, a).as_bytes()))
                }).unwrap_or_default();
                hash_new < hash_cur
            });
            if is_better {
                best_fee = fee;
                best_agent = Some(agent);
            }
        }

        let Some(agent) = best_agent else {
            // No bids — task stays open until deadline.
            continue;
        };

        // Get nonce for the requester — we only auto-assign if we ARE the requester.
        if task.requester != account { continue; }

        let nonce = get_account_nonce(chain, account);
        let entry = LedgerEntry::AgentTaskAssign {
            task_id:   task.task_id.clone(),
            agent:     agent.clone(),
            fee:       best_fee,
            epoch,
            nonce,
            signed_by: account.to_owned(),
        };
        let sig = sign_entry(signing_key, &entry);
        if let Err(e) = crate::tx::validate_and_apply(chain, &entry, sig.as_deref()) {
            warn!("[agent-worker] auto-assign failed for {}: {}", task.task_id, e);
        } else {
            broadcast(cmd_tx, &entry, sig);
            info!("[agent-worker] auto-assigned {} to {} (fee={})", task.task_id, agent, best_fee);
        }
    }
}

/// Execute tasks assigned to this node.
async fn execute_assigned_tasks(
    chain: &Chain,
    account: &str,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
    signing_key: &Arc<Option<ed25519_dalek::SigningKey>>,
    epoch: u64,
) {
    let tasks = scan_tasks(chain, TaskStatus::Assigned);
    for task in tasks {
        if task.agent.as_deref() != Some(account) { continue; }
        if epoch > task.deadline_epoch {
            warn!("[agent-worker] task {} past deadline, skipping", task.task_id);
            continue;
        }

        match run_task(&task, chain).await {
            Ok(output) => {
                let result_hash = hex::encode(Sha256::digest(
                    format!("{}|{}|{}", task.task_id, output, account).as_bytes()
                ));
                let nonce = get_account_nonce(chain, account);
                let entry = LedgerEntry::AgentTaskSubmit {
                    task_id:     task.task_id.clone(),
                    agent:       account.to_owned(),
                    result_hash: result_hash.clone(),
                    output_cid:  String::new(),
                    epoch,
                    nonce,
                    signed_by:   account.to_owned(),
                };
                let sig = sign_entry(signing_key, &entry);
                if let Err(e) = crate::tx::validate_and_apply(chain, &entry, sig.as_deref()) {
                    error!("[agent-worker] submit failed for {}: {}", task.task_id, e);
                } else {
                    broadcast(cmd_tx, &entry, sig);
                    info!("[agent-worker] submitted result for {} (hash={})", task.task_id, &result_hash[..12]);
                }
            }
            Err(e) => {
                error!("[agent-worker] task execution failed for {}: {}", task.task_id, e);
            }
        }
    }
}

/// Act as verifier for submitted tasks we didn't author.
async fn verify_submitted_tasks(
    chain: &Chain,
    account: &str,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
    signing_key: &Arc<Option<ed25519_dalek::SigningKey>>,
    epoch: u64,
) {
    if std::env::var("BTCPC_AGENT_VERIFIER").map(|v| v == "true" || v == "1").unwrap_or(false)
        || std::env::var("BTCPC_AGENT_WORKER").map(|v| v == "true" || v == "1").unwrap_or(false)
    {
        // proceed
    } else {
        return;
    }

    let tasks = scan_tasks(chain, TaskStatus::Submitted);
    for task in tasks {
        // Don't self-verify.
        if task.agent.as_deref() == Some(account) { continue; }
        // Already committed?
        if task.commits.iter().any(|c| c.verifier == account) { continue; }

        match run_task(&task, chain).await {
            Ok(output) => {
                let result_hash = hex::encode(Sha256::digest(
                    format!("{}|{}|{}", task.task_id, output, account).as_bytes()
                ));
                // Generate a random salt.
                let salt = hex::encode(Sha256::digest(
                    format!("verifier-salt:{}:{}:{}", account, task.task_id, epoch).as_bytes()
                ));
                let commit_hash = hex::encode(Sha256::digest(
                    format!("{}{}", result_hash, salt).as_bytes()
                ));

                // Store pending reveal so we can emit it next epoch.
                let pending_key = format!("agent_verifier_pending:{}:{}", task.task_id, account);
                let _ = chain.store.state_set(&pending_key, &serde_json::to_vec(
                    &serde_json::json!({ "result_hash": result_hash, "salt": salt, "epoch": epoch })
                ).unwrap_or_default());

                let nonce = get_account_nonce(chain, account);
                let entry = LedgerEntry::AgentTaskVerifierCommit {
                    task_id:     task.task_id.clone(),
                    verifier:    account.to_owned(),
                    commit_hash,
                    epoch,
                    nonce,
                    signed_by:   account.to_owned(),
                };
                let sig = sign_entry(signing_key, &entry);
                if let Err(e) = crate::tx::validate_and_apply(chain, &entry, sig.as_deref()) {
                    warn!("[agent-worker] verifier commit failed for {}: {}", task.task_id, e);
                } else {
                    broadcast(cmd_tx, &entry, sig);
                    info!("[agent-worker] committed verification for {}", task.task_id);
                }
            }
            Err(e) => {
                warn!("[agent-worker] verification execution failed for {}: {}", task.task_id, e);
            }
        }
    }
}

/// Reveal committed verifications after the delay window.
async fn reveal_committed_verifications(
    chain: &Chain,
    account: &str,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
    signing_key: &Arc<Option<ed25519_dalek::SigningKey>>,
    epoch: u64,
) {
    // Scan all pending reveals for this account.
    for (key, raw) in chain.store.state_scan_prefix("agent_verifier_pending:").into_iter() {
        // key = agent_verifier_pending:{task_id}:{account}
        let parts: Vec<&str> = key.splitn(3, ':').collect();
        if parts.len() < 3 { continue; }
        let task_id = parts[1];
        let verifier = parts[2];
        if verifier != account { continue; }

        let pending: serde_json::Value = match serde_json::from_slice(&raw) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let committed_epoch = pending["epoch"].as_u64().unwrap_or(0);
        if epoch < committed_epoch + VERIFIER_REVEAL_DELAY_EPOCHS { continue; }

        let result_hash = pending["result_hash"].as_str().unwrap_or("").to_owned();
        let salt = pending["salt"].as_str().unwrap_or("").to_owned();

        let nonce = get_account_nonce(chain, account);
        let entry = LedgerEntry::AgentTaskVerifierReveal {
            task_id:     task_id.to_owned(),
            verifier:    account.to_owned(),
            result_hash,
            salt,
            epoch,
            nonce,
            signed_by:   account.to_owned(),
        };
        let sig = sign_entry(signing_key, &entry);
        match crate::tx::validate_and_apply(chain, &entry, sig.as_deref()) {
            Ok(_) => {
                broadcast(cmd_tx, &entry, sig);
                // Remove the pending record.
                let _ = chain.store.state_delete(&key);
                info!("[agent-worker] revealed verification for {}", task_id);
            }
            Err(e) => {
                warn!("[agent-worker] verifier reveal failed for {}: {}", task_id, e);
                // Remove anyway to avoid retry loops on settled tasks.
                let _ = chain.store.state_delete(&key);
            }
        }
    }
}

/// Execute a task via Ollama.  If the task declares allowed tools, runs a
/// ReAct loop (up to 8 iterations); otherwise falls back to a single-shot
/// /api/generate call.
async fn run_task(task: &AgentTask, chain: &Chain) -> anyhow::Result<String> {
    let ollama_url = std::env::var("OLLAMA_URL")
        .unwrap_or_else(|_| "http://localhost:11434".to_owned());
    let model = std::env::var("BTCPC_MODEL")
        .unwrap_or_else(|_| "qwen2.5:0.5b".to_owned());

    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(OLLAMA_TIMEOUT_SECS))
        .build()?;

    if task.tools_allowed.is_empty() {
        let resp = http.post(format!("{}/api/generate", ollama_url))
            .json(&serde_json::json!({
                "model":  model,
                "prompt": task.description,
                "stream": false,
            }))
            .send().await?
            .json::<serde_json::Value>().await?;
        return Ok(resp["response"].as_str().unwrap_or("").to_owned());
    }

    // ReAct loop: build system prompt, then alternate model calls with tool dispatch.
    let system = agent_tools::build_system_prompt(&task.tools_allowed);
    let mut messages = vec![
        serde_json::json!({ "role": "system",  "content": system }),
        serde_json::json!({ "role": "user",    "content": &task.description }),
    ];

    for _ in 0..8usize {
        let resp = http.post(format!("{}/api/chat", ollama_url))
            .json(&serde_json::json!({
                "model":    model,
                "messages": messages,
                "stream":   false,
            }))
            .send().await?
            .json::<serde_json::Value>().await?;

        let content = resp["message"]["content"].as_str().unwrap_or("").to_owned();

        if let Some((tool_name, args)) = agent_tools::parse_tool_call(&content) {
            messages.push(serde_json::json!({ "role": "assistant", "content": &content }));
            let result = agent_tools::dispatch(&tool_name, &args, chain).await;
            let result_text = serde_json::to_string(&result.value).unwrap_or_default();
            messages.push(serde_json::json!({
                "role":    "user",
                "content": format!("[tool:{}] {}", tool_name, result_text),
            }));
            info!("[agent-worker] tool {} ok={}", tool_name, result.ok);
        } else {
            return Ok(content);
        }
    }

    // Exhausted iterations — return last assistant message.
    Ok(messages.iter().rev()
        .find(|m| m["role"].as_str() == Some("assistant"))
        .and_then(|m| m["content"].as_str())
        .unwrap_or("")
        .to_owned())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn scan_tasks(chain: &Chain, status: TaskStatus) -> Vec<AgentTask> {
    chain.store.state_scan_prefix("agent_task:")
        .into_iter()
        .filter_map(|(_, raw)| serde_json::from_slice::<AgentTask>(&raw).ok())
        .filter(|t| t.status == status)
        .collect()
}

fn get_account_nonce(chain: &Chain, account: &str) -> u64 {
    chain.store.get_account(account).ok().flatten()
        .and_then(|s| s["nonce"].as_u64())
        .unwrap_or(0)
}

fn sign_entry(
    signing_key: &Arc<Option<ed25519_dalek::SigningKey>>,
    entry: &LedgerEntry,
) -> Option<String> {
    signing_key.as_ref().as_ref().map(|sk| {
        use ed25519_dalek::Signer;
        let bytes = serde_json::to_vec(entry).unwrap_or_default();
        hex::encode(sk.sign(&bytes).to_bytes())
    })
}

fn broadcast(
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
    entry: &LedgerEntry,
    sig: Option<String>,
) {
    if let Ok(data) = serde_json::to_vec(&serde_json::json!({ "entry": entry, "sig": sig })) {
        let _ = cmd_tx.try_send(NetCmd::Broadcast { topic: "btcpc/entries", data });
    }
}

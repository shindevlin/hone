//! Agentic task marketplace — verified third-party consensus model.
//!
//! Flow:
//!   1. User deposits HONE into agent_credit balance (AgentCreditDeposit).
//!   2. User posts a task (AgentTaskPost) — max_fee reserved from credit.
//!   3. Agents bid during bid_window_epochs (AgentTaskBid).
//!   4. Requester assigns to an agent (AgentTaskAssign), or auto-assigns to
//!      lowest bidder after bid window.
//!   5. Agent executes and submits result hash (AgentTaskSubmit).
//!   6. Verifiers commit-then-reveal (AgentTaskVerifierCommit / Reveal).
//!   7. When min_verifiers majority is reached, AgentTaskSettle fires:
//!        agent fee paid, verifiers split verifier_cut, remainder → credit.
//!
//! State keys:
//!   agent_credit:{account}       — u64 available balance
//!   agent_task:{task_id}         — AgentTask JSON
//!   agent_task_bid:{task_id}:{agent} — u64 proposed fee

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use hone_types::{LedgerEntry, NATIVE_TOKEN};
use crate::chain::Chain;

const VERIFIER_FEE_PCT: u64 = 10; // verifiers collectively get 10% of agreed fee
const TASK_EXPIRY_BUFFER_EPOCHS: u64 = 10;

// ── State structs ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Open,       // accepting bids
    Assigned,   // agent claimed, working
    Submitted,  // agent submitted result_hash
    Verifying,  // collecting verifier reveals
    Settled,    // paid out
    Expired,    // deadline passed without settlement
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub task_id:           String,
    pub requester:         String,
    pub description:       String,
    pub tools_allowed:     Vec<String>,
    pub max_fee:           u64,
    pub agreed_fee:        u64,
    pub min_verifiers:     u32,
    pub bid_window_epochs: u64,
    pub deadline_epoch:    u64,
    pub posted_epoch:      u64,
    pub status:            TaskStatus,
    pub agent:             Option<String>,
    pub result_hash:       Option<String>,
    pub output_cid:        Option<String>,
    pub commits:           Vec<VerifierCommit>,
    pub reveals:           Vec<VerifierReveal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierCommit {
    pub verifier:    String,
    pub commit_hash: String,
    pub epoch:       u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifierReveal {
    pub verifier:    String,
    pub result_hash: String,
    pub salt:        String,
}

// ── Key helpers ───────────────────────────────────────────────────────────────

pub fn credit_key(account: &str) -> String { format!("agent_credit:{}", account) }
pub fn task_key(task_id: &str) -> String { format!("agent_task:{}", task_id) }
pub fn bid_key(task_id: &str, agent: &str) -> String { format!("agent_task_bid:{}:{}", task_id, agent) }

fn load_task(chain: &Chain, task_id: &str) -> Result<AgentTask> {
    let raw = chain.store.state_get(&task_key(task_id))
        .ok_or_else(|| anyhow::anyhow!("task '{}' not found", task_id))?;
    Ok(serde_json::from_slice(&raw)?)
}

fn save_task(chain: &Chain, task: &AgentTask) -> Result<()> {
    chain.store.state_set(&task_key(&task.task_id), &serde_json::to_vec(task)?)
        .map_err(|e| anyhow::anyhow!(e))
}

pub fn get_credit(chain: &Chain, account: &str) -> u64 {
    chain.store.state_get(&credit_key(account))
        .and_then(|b| serde_json::from_slice::<u64>(&b).ok())
        .unwrap_or(0)
}

fn set_credit(chain: &Chain, account: &str, amount: u64) -> Result<()> {
    chain.store.state_set(&credit_key(account), &serde_json::to_vec(&amount)?)
        .map_err(|e| anyhow::anyhow!(e))
}

// ── Apply functions ───────────────────────────────────────────────────────────

pub fn apply_credit_deposit(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::AgentCreditDeposit { account, amount, .. } = entry
        else { bail!("wrong entry type") };

    chain.store.debit(account, NATIVE_TOKEN, *amount)?;
    let new_credit = get_credit(chain, account).saturating_add(*amount);
    set_credit(chain, account, new_credit)?;
    info!("[agent-task] credit deposit: {} += {} hunits", account, amount);
    Ok(())
}

pub fn apply_credit_withdraw(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::AgentCreditWithdraw { account, amount, .. } = entry
        else { bail!("wrong entry type") };

    let credit = get_credit(chain, account);
    anyhow::ensure!(credit >= *amount, "insufficient agent credit (have {}, need {})", credit, amount);
    set_credit(chain, account, credit - amount)?;
    chain.store.credit(account, NATIVE_TOKEN, *amount)?;
    info!("[agent-task] credit withdraw: {} -= {} hunits", account, amount);
    Ok(())
}

pub fn apply_task_post(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::AgentTaskPost {
        task_id, requester, description, tools_allowed,
        max_fee, min_verifiers, bid_window_epochs, deadline_epoch, epoch, ..
    } = entry else { bail!("wrong entry type") };

    anyhow::ensure!(
        chain.store.state_get(&task_key(task_id)).is_none(),
        "task '{}' already exists", task_id
    );

    let credit = get_credit(chain, requester);
    anyhow::ensure!(credit >= *max_fee, "insufficient agent credit (have {}, need {})", credit, max_fee);

    set_credit(chain, requester, credit - max_fee)?;

    let task = AgentTask {
        task_id:           task_id.clone(),
        requester:         requester.clone(),
        description:       description.clone(),
        tools_allowed:     tools_allowed.clone(),
        max_fee:           *max_fee,
        agreed_fee:        0,
        min_verifiers:     *min_verifiers,
        bid_window_epochs: *bid_window_epochs,
        deadline_epoch:    *deadline_epoch,
        posted_epoch:      *epoch,
        status:            TaskStatus::Open,
        agent:             None,
        result_hash:       None,
        output_cid:        None,
        commits:           vec![],
        reveals:           vec![],
    };
    save_task(chain, &task)?;
    info!("[agent-task] posted: {} by {} (max_fee={})", task_id, requester, max_fee);
    Ok(())
}

pub fn apply_task_bid(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::AgentTaskBid { task_id, agent, proposed_fee, epoch, .. } = entry
        else { bail!("wrong entry type") };

    let task = load_task(chain, task_id)?;
    anyhow::ensure!(task.status == TaskStatus::Open, "task is not accepting bids");
    anyhow::ensure!(*proposed_fee <= task.max_fee, "proposed_fee exceeds max_fee");
    anyhow::ensure!(
        *epoch <= task.posted_epoch + task.bid_window_epochs,
        "bid window has closed"
    );

    chain.store.state_set(&bid_key(task_id, agent), &serde_json::to_vec(proposed_fee)?)
        .map_err(|e| anyhow::anyhow!(e))?;
    info!("[agent-task] bid: {} on task {} for {} hunits", agent, task_id, proposed_fee);
    Ok(())
}

pub fn apply_task_assign(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::AgentTaskAssign { task_id, agent, fee, .. } = entry
        else { bail!("wrong entry type") };

    let mut task = load_task(chain, task_id)?;
    anyhow::ensure!(task.status == TaskStatus::Open, "task is not open for assignment");
    anyhow::ensure!(*fee <= task.max_fee, "fee exceeds max_fee");

    // Verify agent actually bid this amount (or less).
    let bid_raw = chain.store.state_get(&bid_key(task_id, agent));
    if let Some(raw) = bid_raw {
        let bid_fee: u64 = serde_json::from_slice(&raw).unwrap_or(u64::MAX);
        anyhow::ensure!(*fee <= bid_fee, "assigned fee exceeds agent's bid");
    } else {
        bail!("agent '{}' has not bid on task '{}'", agent, task_id);
    }

    task.agent = Some(agent.clone());
    task.agreed_fee = *fee;
    task.status = TaskStatus::Assigned;
    save_task(chain, &task)?;
    info!("[agent-task] assigned: {} to agent {} (fee={})", task_id, agent, fee);
    Ok(())
}

pub fn apply_task_submit(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::AgentTaskSubmit { task_id, agent, result_hash, output_cid, .. } = entry
        else { bail!("wrong entry type") };

    let mut task = load_task(chain, task_id)?;
    anyhow::ensure!(task.status == TaskStatus::Assigned, "task is not in Assigned state");
    anyhow::ensure!(
        task.agent.as_deref() == Some(agent.as_str()),
        "only the assigned agent can submit"
    );

    task.result_hash = Some(result_hash.clone());
    task.output_cid = Some(output_cid.clone());
    task.status = TaskStatus::Submitted;
    save_task(chain, &task)?;
    info!("[agent-task] submitted: {} by {} (hash={})", task_id, agent, &result_hash[..12]);
    Ok(())
}

pub fn apply_verifier_commit(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::AgentTaskVerifierCommit { task_id, verifier, commit_hash, epoch, .. } = entry
        else { bail!("wrong entry type") };

    let mut task = load_task(chain, task_id)?;
    anyhow::ensure!(
        task.status == TaskStatus::Submitted || task.status == TaskStatus::Verifying,
        "task is not awaiting verification"
    );
    anyhow::ensure!(
        task.commits.iter().all(|c| &c.verifier != verifier),
        "verifier '{}' already committed", verifier
    );

    task.commits.push(VerifierCommit {
        verifier: verifier.clone(),
        commit_hash: commit_hash.clone(),
        epoch: *epoch,
    });
    task.status = TaskStatus::Verifying;
    save_task(chain, &task)?;
    Ok(())
}

pub fn apply_verifier_reveal(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::AgentTaskVerifierReveal { task_id, verifier, result_hash, salt, .. } = entry
        else { bail!("wrong entry type") };

    let mut task = load_task(chain, task_id)?;
    anyhow::ensure!(task.status == TaskStatus::Verifying, "task is not in Verifying state");

    // Verify the reveal matches the commit.
    let expected_commit = hex::encode(Sha256::digest(
        format!("{}{}", result_hash, salt).as_bytes()
    ));
    let commit = task.commits.iter().find(|c| &c.verifier == verifier)
        .ok_or_else(|| anyhow::anyhow!("no commit found for verifier '{}'", verifier))?;
    anyhow::ensure!(
        commit.commit_hash == expected_commit,
        "reveal does not match commit for verifier '{}'", verifier
    );
    anyhow::ensure!(
        task.reveals.iter().all(|r| &r.verifier != verifier),
        "verifier '{}' already revealed", verifier
    );

    task.reveals.push(VerifierReveal {
        verifier: verifier.clone(),
        result_hash: result_hash.clone(),
        salt: salt.clone(),
    });
    save_task(chain, &task)?;
    Ok(())
}

/// Called by finalize / epoch sweep once min_verifiers reveals are in.
pub fn try_settle(chain: &Chain, task_id: &str, epoch: u64) -> Result<bool> {
    let mut task = load_task(chain, task_id)?;
    if task.status != TaskStatus::Verifying { return Ok(false); }

    let agent = match &task.agent {
        Some(a) => a.clone(),
        None => return Ok(false),
    };
    let agent_hash = match &task.result_hash {
        Some(h) => h.clone(),
        None => return Ok(false),
    };

    // Need at least min_verifiers reveals.
    if task.reveals.len() < task.min_verifiers as usize { return Ok(false); }

    // Tally votes — majority hash wins.
    let mut tally: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for rev in &task.reveals {
        tally.entry(rev.result_hash.clone()).or_default().push(rev.verifier.clone());
    }
    let (winning_hash, winners) = tally.into_iter()
        .max_by_key(|(_, v)| v.len())
        .unwrap();

    let agent_correct = agent_hash == winning_hash;
    let verifier_cut_total = task.agreed_fee.saturating_mul(VERIFIER_FEE_PCT) / 100;
    let agent_fee = if agent_correct {
        task.agreed_fee.saturating_sub(verifier_cut_total)
    } else {
        0
    };
    let per_verifier = if winners.is_empty() {
        0
    } else {
        verifier_cut_total / winners.len() as u64
    };
    let refund = task.max_fee
        .saturating_sub(agent_fee)
        .saturating_sub(per_verifier.saturating_mul(winners.len() as u64));

    // Pay agent.
    if agent_fee > 0 {
        let _ = chain.store.credit(&agent, NATIVE_TOKEN, agent_fee);
    }
    // Pay verifiers.
    for v in &winners {
        if per_verifier > 0 {
            let _ = chain.store.credit(v, NATIVE_TOKEN, per_verifier);
        }
    }
    // Refund unused credit.
    if refund > 0 {
        let credit = get_credit(chain, &task.requester).saturating_add(refund);
        let _ = set_credit(chain, &task.requester, credit);
    }

    // Emit settle entry (best-effort, errors are non-fatal).
    let settle = LedgerEntry::AgentTaskSettle {
        task_id:      task_id.to_owned(),
        agent:        agent.clone(),
        winning_hash: winning_hash.clone(),
        fee_paid:     agent_fee,
        verifier_cut: per_verifier,
        epoch,
    };
    chain.apply_entry(&settle).ok();

    task.status = TaskStatus::Settled;
    save_task(chain, &task)?;
    info!(
        "[agent-task] settled: {} agent={} correct={} fee={} refund={}",
        task_id, agent, agent_correct, agent_fee, refund
    );
    Ok(true)
}

pub fn apply_settle(_chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::AgentTaskSettle { .. } = entry else { bail!("wrong entry type") };
    // Payments already applied in try_settle; this is a no-op record pass.
    Ok(())
}

/// Sweep expired tasks and return max_fee to requester's credit balance.
pub fn sweep_expired(chain: &Chain, epoch: u64) {
    let prefix = "agent_task:";
    for (key, raw) in chain.store.state_scan_prefix(prefix).into_iter() {
        let Ok(mut task) = serde_json::from_slice::<AgentTask>(&raw) else { continue };
        if task.status == TaskStatus::Settled
            || task.status == TaskStatus::Expired
            || task.status == TaskStatus::Cancelled
        {
            continue;
        }
        if epoch <= task.deadline_epoch + TASK_EXPIRY_BUFFER_EPOCHS { continue; }

        // Auto-settle if verifiers are in.
        if task.status == TaskStatus::Verifying
            && task.reveals.len() >= task.min_verifiers as usize
        {
            if try_settle(chain, &task.task_id, epoch).unwrap_or(false) {
                continue;
            }
        }

        // Refund escrow.
        let credit = get_credit(chain, &task.requester).saturating_add(task.max_fee);
        let _ = set_credit(chain, &task.requester, credit);
        task.status = TaskStatus::Expired;
        task.max_fee = 0;
        let _ = chain.store.state_set(&key, &serde_json::to_vec(&task).unwrap_or_default());
        warn!("[agent-task] expired: {} (requester={})", task.task_id, task.requester);
    }
}

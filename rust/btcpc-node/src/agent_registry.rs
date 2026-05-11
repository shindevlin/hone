//! Agent registry — on-chain advertisement of agent capabilities and pricing.
//!
//! Agents register their tools, model, and minimum fee. Requesters query the
//! registry to find agents before posting tasks.
//!
//! State key: `agent_registry:{account}` → AgentRegistration JSON

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use btcpc_types::LedgerEntry;
use crate::chain::Chain;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRegistration {
    pub account:          String,
    pub name:             String,
    pub description:      String,
    pub tools:            Vec<String>,
    pub model:            String,
    pub min_fee:          u64,
    pub registered_epoch: u64,
    pub updated_epoch:    u64,
    pub tasks_completed:  u64,
    pub tasks_verified:   u64,
    pub active:           bool,
}

pub fn registry_key(account: &str) -> String { format!("agent_registry:{}", account) }

pub fn apply_register(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::AgentRegister {
        account, name, description, tools, model, min_fee, epoch, ..
    } = entry else { bail!("wrong entry type") };

    let existing = chain.store.state_get(&registry_key(account))
        .and_then(|b| serde_json::from_slice::<AgentRegistration>(&b).ok());

    let reg = AgentRegistration {
        account:          account.clone(),
        name:             name.clone(),
        description:      description.clone(),
        tools:            tools.clone(),
        model:            model.clone(),
        min_fee:          *min_fee,
        registered_epoch: existing.as_ref().map(|r| r.registered_epoch).unwrap_or(*epoch),
        updated_epoch:    *epoch,
        tasks_completed:  existing.as_ref().map(|r| r.tasks_completed).unwrap_or(0),
        tasks_verified:   existing.as_ref().map(|r| r.tasks_verified).unwrap_or(0),
        active:           true,
    };

    chain.store.state_set(&registry_key(account), &serde_json::to_vec(&reg)?)
        .map_err(|e| anyhow::anyhow!(e))?;
    info!("[agent-registry] registered: {} ({} tools)", account, tools.len());
    Ok(())
}

pub fn apply_deregister(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::AgentDeregister { account, .. } = entry
        else { bail!("wrong entry type") };

    let raw = chain.store.state_get(&registry_key(account))
        .ok_or_else(|| anyhow::anyhow!("agent '{}' not registered", account))?;
    let mut reg: AgentRegistration = serde_json::from_slice(&raw)?;
    anyhow::ensure!(reg.active, "agent already deregistered");

    reg.active = false;
    chain.store.state_set(&registry_key(account), &serde_json::to_vec(&reg)?)
        .map_err(|e| anyhow::anyhow!(e))?;
    info!("[agent-registry] deregistered: {}", account);
    Ok(())
}

/// Called by agent_task settle logic to increment completion counters.
#[allow(dead_code)]
pub fn record_task_completed(chain: &Chain, account: &str) {
    if let Some(raw) = chain.store.state_get(&registry_key(account)) {
        if let Ok(mut reg) = serde_json::from_slice::<AgentRegistration>(&raw) {
            reg.tasks_completed += 1;
            let _ = chain.store.state_set(&registry_key(account), &serde_json::to_vec(&reg).unwrap_or_default());
        }
    }
}

/// Called by agent_task settle logic to increment verifier counters.
#[allow(dead_code)]
pub fn record_verification(chain: &Chain, account: &str) {
    if let Some(raw) = chain.store.state_get(&registry_key(account)) {
        if let Ok(mut reg) = serde_json::from_slice::<AgentRegistration>(&raw) {
            reg.tasks_verified += 1;
            let _ = chain.store.state_set(&registry_key(account), &serde_json::to_vec(&reg).unwrap_or_default());
        }
    }
}

/// List all active registrations, optionally filtered by tool.
pub fn list_agents(chain: &Chain, tool_filter: Option<&str>) -> Vec<AgentRegistration> {
    chain.store.state_scan_prefix("agent_registry:")
        .into_iter()
        .filter_map(|(_, raw)| serde_json::from_slice::<AgentRegistration>(&raw).ok())
        .filter(|r| r.active)
        .filter(|r| {
            tool_filter.map(|t| r.tools.iter().any(|rt| rt == t)).unwrap_or(true)
        })
        .collect()
}

//! Private authorization — M-of-N threshold signing for high-value transfers.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::chain::Chain;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthGroup {
    pub group_id: String,
    pub members: Vec<MemberKey>,
    pub threshold_n: u32,
    pub threshold_m: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberKey {
    pub account: String,
    pub pubkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRecord {
    pub group_id: String,
    pub tx_hash: String,
    pub approvals: Vec<String>,
    pub finalized: bool,
}

fn group_key(id: &str) -> String { format!("private_auth_group:{}", id) }
fn approval_key(tx_hash: &str) -> String { format!("private_auth_approval:{}", tx_hash) }

pub fn apply_enroll(
    chain: &Chain,
    group_id: &str,
    member: &str,
    member_pubkey: &str,
    threshold_n: u32,
    threshold_m: u32,
) -> Result<()> {
    anyhow::ensure!(threshold_m <= threshold_n && threshold_m >= 1,
        "invalid threshold: M={} N={}", threshold_m, threshold_n);

    let mut group: AuthGroup = chain.store.state_get(&group_key(group_id))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(AuthGroup {
            group_id: group_id.to_string(),
            members: vec![],
            threshold_n,
            threshold_m,
        });

    if !group.members.iter().any(|m| m.account == member) {
        group.members.push(MemberKey {
            account: member.to_string(),
            pubkey: member_pubkey.to_string(),
        });
    }

    // Update thresholds if caller is amending
    group.threshold_n = threshold_n;
    group.threshold_m = threshold_m;

    chain.store.state_set(&group_key(group_id), &serde_json::to_vec(&group)?)?;
    Ok(())
}

pub fn apply_approve(
    chain: &Chain,
    group_id: &str,
    tx_hash: &str,
    approver: &str,
) -> Result<bool> {
    let raw = chain.store.state_get(&group_key(group_id))
        .ok_or_else(|| anyhow::anyhow!("group '{}' not found", group_id))?;
    let group: AuthGroup = serde_json::from_slice(&raw)?;

    anyhow::ensure!(group.members.iter().any(|m| m.account == approver),
        "approver is not a member of group '{}'", group_id);

    let mut record: ApprovalRecord = chain.store.state_get(&approval_key(tx_hash))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(ApprovalRecord {
            group_id: group_id.to_string(),
            tx_hash: tx_hash.to_string(),
            approvals: vec![],
            finalized: false,
        });

    anyhow::ensure!(!record.finalized, "approval already finalized");

    if !record.approvals.contains(&approver.to_string()) {
        record.approvals.push(approver.to_string());
    }

    let threshold_met = record.approvals.len() as u32 >= group.threshold_m;
    if threshold_met {
        record.finalized = true;
    }

    chain.store.state_set(&approval_key(tx_hash), &serde_json::to_vec(&record)?)?;
    Ok(threshold_met)
}

pub fn approval_status(chain: &Chain, tx_hash: &str) -> serde_json::Value {
    let record: Option<ApprovalRecord> = chain.store.state_get(&approval_key(tx_hash))
        .and_then(|b| serde_json::from_slice(&b).ok());
    match record {
        Some(r) => serde_json::json!({
            "tx_hash": r.tx_hash,
            "group_id": r.group_id,
            "approvals": r.approvals,
            "finalized": r.finalized,
        }),
        None => serde_json::json!({ "tx_hash": tx_hash, "approvals": [], "finalized": false }),
    }
}

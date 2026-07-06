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

#[cfg(test)]
mod tests {
    use super::*;
    use hone_types::LedgerEntry;
    use tempfile::TempDir;

    fn make_chain() -> (Chain, TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(
            store, "test".to_string(), "hone-test".to_string(),
        );
        (chain, dir)
    }

    fn fund(chain: &Chain, account: &str, amount: u64) {
        chain.apply_entry(&LedgerEntry::AccountCreate {
            account: account.to_string(),
            keys: Default::default(),
            chain_proofs: vec![],
            epoch: 0,
            funded_by: None,
            machine_fingerprint: None,
        }).ok();
        chain.apply_entry(&LedgerEntry::GenesisAlloc {
            account: account.to_string(),
            amount,
            token: hone_types::NATIVE_TOKEN.to_string(),
        }).unwrap();
    }

    #[test]
    fn enroll_stores_member() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", 0);

        apply_enroll(&chain, "group1", "alice", "pubkey_alice", 3, 2).unwrap();

        let raw = chain.store.state_get(&group_key("group1")).unwrap();
        let group: AuthGroup = serde_json::from_slice(&raw).unwrap();
        assert_eq!(group.group_id, "group1");
        assert_eq!(group.members.len(), 1);
        assert_eq!(group.members[0].account, "alice");
        assert_eq!(group.threshold_m, 2);
        assert_eq!(group.threshold_n, 3);
    }

    #[test]
    fn approve_records_vote() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", 0);
        fund(&chain, "bob", 0);

        apply_enroll(&chain, "group2", "alice", "pk_alice", 3, 2).unwrap();
        apply_enroll(&chain, "group2", "bob", "pk_bob", 3, 2).unwrap();

        apply_approve(&chain, "group2", "tx_hash_1", "alice").unwrap();

        let status = approval_status(&chain, "tx_hash_1");
        let approvals = status["approvals"].as_array().unwrap();
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0].as_str().unwrap(), "alice");
        assert_eq!(status["finalized"].as_bool().unwrap(), false);
    }

    #[test]
    fn approve_reaches_threshold() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", 0);
        fund(&chain, "bob", 0);
        fund(&chain, "carol", 0);

        // 3 members, threshold_m = 2
        apply_enroll(&chain, "group3", "alice", "pk_a", 3, 2).unwrap();
        apply_enroll(&chain, "group3", "bob", "pk_b", 3, 2).unwrap();
        apply_enroll(&chain, "group3", "carol", "pk_c", 3, 2).unwrap();

        apply_approve(&chain, "group3", "tx_hash_2", "alice").unwrap();
        // First approval: not yet finalized
        let s = approval_status(&chain, "tx_hash_2");
        assert_eq!(s["finalized"].as_bool().unwrap(), false);

        let reached = apply_approve(&chain, "group3", "tx_hash_2", "bob").unwrap();
        // Second approval: threshold_m=2 reached
        assert!(reached, "threshold should be reached with 2 approvals");

        let s = approval_status(&chain, "tx_hash_2");
        assert_eq!(s["finalized"].as_bool().unwrap(), true);
    }

    #[test]
    fn approve_unknown_group_fails() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", 0);
        let res = apply_approve(&chain, "nonexistent_group", "tx_hash_3", "alice");
        assert!(res.is_err(), "approving on non-existent group should fail");
    }
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

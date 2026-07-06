//! Lightweight chain state for the Android micronode.
//! Applies LedgerEntries to the sled store and tracks epoch progress.

use std::sync::Arc;
use anyhow::{bail, Result};
use parking_lot::RwLock;
use tracing::warn;

use hone_types::{
    LedgerEntry, NATIVE_TOKEN,
    block_reward_at, era, RECYCLE_ERA, RECYCLE_REWARD_RATE, RECYCLE_REWARD_DENOM,
    RECYCLE_FUND_ACCOUNT, DEVICE_CLAIM_OVERBID_NUM, DEVICE_CLAIM_OVERBID_DENOM,
    OVERCLAIM_STAKER_SHARE_BPS,
};

use crate::store::Store;

fn token_approved(store: &Store, to: &str, token: &str, from: &str) -> bool {
    store.get_json::<serde_json::Value>(&format!("token_approval:{}:{}:{}", to, token, from)).is_some()
        || store.get_json::<serde_json::Value>(&format!("token_approval:{}:*:{}", to, from)).is_some()
        || store.get_json::<serde_json::Value>(&format!("token_approval:{}:{}:*", to, token)).is_some()
        || store.get_json::<serde_json::Value>(&format!("token_approval:{}:*:*", to)).is_some()
}

fn spam_gate_fee(store: &Store, to: &str, token: &str) -> Option<(u64, String)> {
    for key in [
        format!("spam_gate:{}:{}", to, token),
        format!("spam_gate:{}:*", to),
    ] {
        if let Some(j) = store.get_json::<serde_json::Value>(&key) {
            if let Some(fee) = j["fee"].as_u64() {
                let fee_token = j["fee_token"].as_str()
                    .unwrap_or(NATIVE_TOKEN).to_owned();
                return Some((fee, fee_token));
            }
        }
    }
    None
}

pub struct Chain {
    pub store:         Store,
    pub node_id:       String,
    pub chain_id:      String,
    pub current_epoch: RwLock<u64>,
}

impl Chain {
    pub fn new(store: Store, node_id: String, chain_id: String) -> Self {
        let epoch = store.latest_epoch().unwrap_or(0);
        Chain {
            store,
            node_id,
            chain_id,
            current_epoch: RwLock::new(epoch),
        }
    }

    pub fn current_epoch(&self) -> u64 {
        *self.current_epoch.read()
    }

    pub fn apply_entry(&self, entry: &LedgerEntry) -> Result<()> {
        use LedgerEntry::*;
        match entry {
            GenesisAlloc { account, amount, token, .. } => {
                self.store.set_balance(account, token, *amount);
            }
            Transfer { from, to, amount, token, nonce, .. } => {
                let expected = self.store.get_nonce(from) + 1;
                if *nonce != expected {
                    bail!("bad nonce: got {} expected {}", nonce, expected);
                }
                if !self.store.debit(from, token, *amount) {
                    bail!("insufficient balance");
                }
                self.store.credit(to, token, *amount);
                self.store.increment_nonce(from);
            }
            Mine { miner, epoch, model, input_tokens, output_tokens, tool_calls, hw_tier, output_hash, .. } => {
                let key = format!("mine:{}:{}", epoch, miner);
                let rec = serde_json::json!({
                    "miner": miner, "epoch": epoch,
                    "model": model, "input_tokens": input_tokens,
                    "output_tokens": output_tokens, "tool_calls": tool_calls,
                    "hw_tier": hw_tier, "output_hash": output_hash,
                });
                let _ = self.store.set_json(&key, &rec);
            }
            MineReward { miner, amount, epoch } => {
                let reward = if era(*epoch) >= RECYCLE_ERA {
                    let fund = self.store.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN);
                    ((fund as u128 * RECYCLE_REWARD_RATE) / RECYCLE_REWARD_DENOM) as u64
                } else {
                    *amount
                };
                self.store.credit(miner, NATIVE_TOKEN, reward);
                // Drain recycle fund if in recycle era.
                if era(*epoch) >= RECYCLE_ERA {
                    self.store.debit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, reward);
                }
            }
            ClockReward { node_id, amount, .. } => {
                self.store.credit(node_id, NATIVE_TOKEN, *amount);
            }
            EpochSeal { epoch, .. } => {
                let mut cur = self.current_epoch.write();
                if *epoch > *cur { *cur = *epoch; }
            }
            EpochFinalize { epoch, .. } => {
                let mut cur = self.current_epoch.write();
                if *epoch > *cur { *cur = *epoch; }
            }
            InferenceJobPost { job_id, input_hash, model, .. } => {
                let rec = serde_json::json!({
                    "job_id":     job_id,
                    "input_hash": input_hash,
                    "model":      model,
                    "status":     "posted",
                });
                let _ = self.store.set_json(&format!("infer_job:{}", job_id), &rec);
            }
            InferenceJobAward { job_id, winner, .. } => {
                let key = format!("infer_job:{}", job_id);
                if let Some(mut rec) = self.store.get_json::<serde_json::Value>(&key) {
                    rec["winner"] = serde_json::Value::String(winner.clone());
                    rec["status"] = serde_json::Value::String("awarded".into());
                    let _ = self.store.set_json(&key, &rec);
                }
            }
            InferenceJobComplete { job_id, worker, result_hash, .. } => {
                let key = format!("infer_job:{}", job_id);
                if let Some(mut rec) = self.store.get_json::<serde_json::Value>(&key) {
                    rec["winner"]      = serde_json::Value::String(worker.clone());
                    rec["result_hash"] = serde_json::Value::String(result_hash.clone());
                    rec["status"]      = serde_json::Value::String("complete".into());
                    let _ = self.store.set_json(&key, &rec);
                }
            }
            InferenceJobPay { worker, worker_amount, .. } => {
                self.store.credit(worker, NATIVE_TOKEN, *worker_amount);
            }
            // Stake / Unstake: update stake balance
            Stake { account, amount, .. } => {
                let cur = self.store.get_stake(account);
                self.store.set_stake(account, cur.saturating_add(*amount));
                if !self.store.debit(account, NATIVE_TOKEN, *amount) {
                    bail!("insufficient balance to stake");
                }
            }
            Unstake { account, amount, .. } => {
                let cur = self.store.get_stake(account);
                if cur < *amount { bail!("insufficient stake"); }
                self.store.set_stake(account, cur - *amount);
                self.store.credit(account, NATIVE_TOKEN, *amount);
            }

            // Track per-epoch verifier value_score totals for pool reward scoring.
            InferenceJobVerify { job_id: _, verifier, verdict, value_score, epoch, .. } => {
                let key = format!("infer_verify:{}:{}", epoch, verifier);
                let prev_total: u64 = self.store.get_json::<serde_json::Value>(&key)
                    .and_then(|j| j["value_score_total"].as_u64())
                    .unwrap_or(0);
                // Only count approved verdicts toward the pool score
                let add = if verdict == "approved" { *value_score } else { 0 };
                let _ = self.store.set_json(&key, &serde_json::json!({
                    "verifier": verifier, "epoch": epoch,
                    "value_score_total": prev_total + add,
                }));
            }

            // Track storage heartbeats (include query_count for bonus).
            StorageHeartbeat { node_id, epoch, bytes_proven, query_count, .. } => {
                let key = format!("storage_beat:{}:{}", epoch, node_id);
                let _ = self.store.set_json(&key, &serde_json::json!({
                    "node_id": node_id, "epoch": epoch,
                    "bytes_proven": bytes_proven, "query_count": query_count,
                }));
            }

            // Track sensor commits per sensor_id with sensor_type for scored rewards.
            SensorDataCommit { sensor_id, owner, epoch, reading_count, sensor_type, .. } => {
                let key = format!("sensor_commit:{}:{}", epoch, sensor_id);
                let _ = self.store.set_json(&key, &serde_json::json!({
                    "sensor_id": sensor_id, "owner": owner, "epoch": epoch,
                    "reading_count": reading_count, "sensor_type": sensor_type,
                }));
            }

            // Layer B epoch rewards — credit account
            StorageReward { node_id, amount, .. } => {
                self.store.credit(node_id, NATIVE_TOKEN, *amount);
            }
            SensorReward { node_id, amount, .. } => {
                self.store.credit(node_id, NATIVE_TOKEN, *amount);
            }
            VerifierReward { node_id, amount, .. } => {
                self.store.credit(node_id, NATIVE_TOKEN, *amount);
            }
            ServiceReward { node_id, amount, .. } => {
                self.store.credit(node_id, NATIVE_TOKEN, *amount);
            }
            MempoolReward { operator, amount, .. } => {
                self.store.credit(operator, NATIVE_TOKEN, *amount);
            }

            // Service heartbeat
            ServiceHeartbeat { node_id, epoch, container_hours, .. } => {
                let key = format!("service_beat:{}:{}", epoch, node_id);
                let _ = self.store.set_json(&key, &serde_json::json!({
                    "node_id": node_id, "epoch": epoch, "container_hours": container_hours,
                }));
            }

            // Mempool operator heartbeat
            MempoolHeartbeat { operator, epoch, propagation_latency_ms, entries_relayed, .. } => {
                let key = format!("mempool_beat:{}:{}", epoch, operator);
                let _ = self.store.set_json(&key, &serde_json::json!({
                    "operator": operator, "epoch": epoch,
                    "latency_ms": propagation_latency_ms,
                    "entries_relayed": entries_relayed,
                }));
            }

            // Device yield stake — tracked so overbid can distribute premium
            DeviceYieldStake { device_serial, staker, amount, .. } => {
                if !self.store.debit(staker, NATIVE_TOKEN, *amount) {
                    bail!("insufficient balance for yield stake");
                }
                let key = format!("yield_stake:{}:{}", device_serial, staker);
                let prev: u64 = self.store.get_json::<serde_json::Value>(&key)
                    .and_then(|j| j["amount"].as_u64()).unwrap_or(0);
                let _ = self.store.set_json(&key, &serde_json::json!({
                    "device_serial": device_serial, "staker": staker,
                    "amount": prev + amount,
                }));
            }

            DeviceYieldUnstake { device_serial, staker, amount, .. } => {
                let key = format!("yield_stake:{}:{}", device_serial, staker);
                let current: u64 = self.store.get_json::<serde_json::Value>(&key)
                    .and_then(|j| j["amount"].as_u64()).unwrap_or(0);
                if current < *amount { bail!("insufficient yield stake"); }
                let remaining = current - amount;
                if remaining == 0 {
                    self.store.delete_state(&key);
                } else {
                    let _ = self.store.set_json(&key, &serde_json::json!({
                        "device_serial": device_serial, "staker": staker, "amount": remaining,
                    }));
                }
                self.store.credit(staker, NATIVE_TOKEN, *amount);
            }

            // Device claim stake — overbid distributes premium to yield stakers
            DeviceClaimStake { device_serial, owner, amount, epoch, .. } => {
                let claim_key = format!("device_claim:{}", device_serial);
                if let Some(j) = self.store.get_json::<serde_json::Value>(&claim_key) {
                    let existing_owner = j["owner"].as_str().unwrap_or("");
                    let existing_stake = j["stake"].as_u64().unwrap_or(0);
                    if !existing_owner.is_empty() && existing_owner != owner.as_str() {
                        let required = ((existing_stake as u128)
                            .saturating_mul(DEVICE_CLAIM_OVERBID_NUM)
                            / DEVICE_CLAIM_OVERBID_DENOM) as u64;
                        if *amount < required {
                            bail!("overbid requires ≥ {} (1.5×)", required);
                        }
                        // Return old claim owner's full principal
                        self.store.credit(existing_owner, NATIVE_TOKEN, existing_stake);

                        // Distribute premium: 50% to yield stakers, 50% to recycle
                        let premium = amount.saturating_sub(existing_stake);
                        if premium > 0 {
                            let staker_share = (premium as u128
                                * OVERCLAIM_STAKER_SHARE_BPS as u128 / 10_000) as u64;
                            let recycle_share = premium.saturating_sub(staker_share);

                            let prefix = format!("yield_stake:{}:", device_serial);
                            let stakers: Vec<(String, u64)> = self.store.scan_prefix(&prefix)
                                .into_iter()
                                .filter_map(|(_, v)| {
                                    let js = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
                                    let s = js["staker"].as_str()?.to_owned();
                                    let a = js["amount"].as_u64().unwrap_or(0);
                                    if a > 0 { Some((s, a)) } else { None }
                                }).collect();

                            let total_yield: u64 = stakers.iter().map(|(_, a)| a).sum();
                            if total_yield > 0 && staker_share > 0 {
                                for (staker, stake_amt) in &stakers {
                                    let payout = (staker_share as u128 * *stake_amt as u128
                                        / total_yield as u128) as u64;
                                    if payout > 0 {
                                        self.store.credit(staker, NATIVE_TOKEN, payout);
                                    }
                                }
                            } else {
                                self.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, staker_share);
                            }
                            if recycle_share > 0 {
                                self.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_share);
                            }
                        }
                    }
                }
                if !self.store.debit(owner, NATIVE_TOKEN, *amount) {
                    bail!("insufficient balance for device claim");
                }
                let _ = self.store.set_json(&claim_key, &serde_json::json!({
                    "device_serial": device_serial, "owner": owner,
                    "stake": amount, "claimed_epoch": epoch,
                }));
            }

            DeviceClaimUnstake { device_serial, owner, .. } => {
                let claim_key = format!("device_claim:{}", device_serial);
                if let Some(j) = self.store.get_json::<serde_json::Value>(&claim_key) {
                    if j["owner"].as_str() == Some(owner.as_str()) {
                        let stake = j["stake"].as_u64().unwrap_or(0);
                        self.store.credit(owner, NATIVE_TOKEN, stake);
                        self.store.delete_state(&claim_key);
                    }
                }
            }

            // Scoped delegation
            DelegationGrant { from, to, capabilities, expires_epoch, .. } => {
                let key = format!("delegation:{}:{}", from, to);
                let _ = self.store.set_json(&key, &serde_json::json!({
                    "from": from, "to": to,
                    "capabilities": capabilities,
                    "expires_epoch": expires_epoch,
                }));
            }
            DelegationRevoke { from, to, .. } => {
                self.store.delete_state(&format!("delegation:{}:{}", from, to));
            }

            // Device yield opt-in
            DeviceYieldOptIn { device_serial, owner, max_stakers, .. } => {
                let _ = self.store.set_json(&format!("yield_config:{}", device_serial), &serde_json::json!({
                    "owner": owner, "max_stakers": max_stakers, "opted_in": true,
                }));
            }
            DeviceYieldOptOut { device_serial, .. } => {
                self.store.delete_state(&format!("yield_config:{}", device_serial));
            }

            // Permissive token model: pre-approved passes through; spam gate or pending
            Transfer { from, to, amount, token, nonce, .. } if token != NATIVE_TOKEN => {
                if token_approved(&self.store, to, token, from) {
                    self.store.credit(to, token, *amount);
                } else if let Some((fee, fee_token)) = spam_gate_fee(&self.store, to, token) {
                    if self.store.get_balance(from, &fee_token) >= fee {
                        self.store.debit(from, &fee_token, fee);
                        self.store.credit(to, &fee_token, fee);
                        self.store.credit(to, token, *amount);
                    }
                    // If sender can't pay the gate, transfer is silently dropped on mobile.
                } else {
                    let pending_key = format!("pending_transfer:{}:{}:{}:{}", to, token, from, nonce);
                    let _ = self.store.set_json(&pending_key, &serde_json::json!({
                        "from": from, "to": to, "token": token, "amount": amount,
                    }));
                }
            }
            SpamGateSet { account, token, fee, fee_token, .. } => {
                let _ = self.store.set_json(
                    &format!("spam_gate:{}:{}", account, token),
                    &serde_json::json!({"fee": fee, "fee_token": fee_token}),
                );
            }
            SpamGateClear { account, token, .. } => {
                self.store.delete_state(&format!("spam_gate:{}:{}", account, token));
            }
            TokenApprove { account, token, from, .. } => {
                let key = format!("token_approval:{}:{}:{}", account, token, from);
                let _ = self.store.set_json(&key, &serde_json::json!(1u8));
            }
            TokenRevoke { account, token, from, .. } => {
                self.store.delete_state(&format!("token_approval:{}:{}:{}", account, token, from));
            }
            TokenAccept { to, token, from, pending_nonce, .. } => {
                let pending_key = format!("pending_transfer:{}:{}:{}:{}", to, token, from, pending_nonce);
                if let Some(j) = self.store.get_json::<serde_json::Value>(&pending_key) {
                    let amount = j["amount"].as_u64().unwrap_or(0);
                    self.store.delete_state(&pending_key);
                    self.store.credit(to, token, amount);
                }
            }
            TokenReject { to, token, from, pending_nonce, .. } => {
                let pending_key = format!("pending_transfer:{}:{}:{}:{}", to, token, from, pending_nonce);
                if let Some(j) = self.store.get_json::<serde_json::Value>(&pending_key) {
                    let amount = j["amount"].as_u64().unwrap_or(0);
                    self.store.delete_state(&pending_key);
                    self.store.credit(from, token, amount);
                }
            }

            // Wallet family — additive-only permanent cross-chain address linkage
            WalletFamilyPublish { account, chains, .. }
            | WalletFamilyAdd { account, chains, .. } => {
                let family_key = format!("wallet_family:{}", account);
                let mut existing: Vec<serde_json::Value> = self.store
                    .get_json::<Vec<serde_json::Value>>(&family_key)
                    .unwrap_or_default();
                for ca in chains {
                    let already = existing.iter().any(|e| {
                        e["chain"].as_str() == Some(&ca.chain)
                            && e["address"].as_str() == Some(&ca.address)
                    });
                    if !already {
                        let rev_key = format!("wallet_addr:{}:{}", ca.chain, ca.address);
                        let _ = self.store.set_json(&rev_key, &serde_json::json!(account));
                        existing.push(serde_json::json!({
                            "chain": ca.chain,
                            "address": ca.address,
                            "derivation_path": ca.derivation_path,
                        }));
                    }
                }
                let _ = self.store.set_json(&family_key, &existing);
            }

            _ => {} // other entries don't affect mobile state
        }
        Ok(())
    }

    pub fn apply_block_entries(&self, entries: &[LedgerEntry]) {
        for e in entries {
            if let Err(err) = self.apply_entry(e) {
                warn!("apply_entry failed: {}", err);
            }
        }
    }
}

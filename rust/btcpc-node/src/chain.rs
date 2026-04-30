//! Chain state machine — applies ledger entries and advances chain state.

use std::sync::Arc;
use anyhow::Result;
use parking_lot::{Mutex, RwLock};
use tracing::{info, warn};
use btcpc_types::{AccountId, LedgerEntry, NATIVE_TOKEN, CLOCK_REWARD_DREAMS, era, RECYCLE_ERA, RECYCLE_FUND_ACCOUNT, TESTNET_FUND_ACCOUNT, DEVICE_CLAIM_OVERBID_NUM, DEVICE_CLAIM_OVERBID_DENOM, OVERCLAIM_STAKER_SHARE_BPS};

use crate::inference;
use crate::store::Store;

/// Returns true if `to` has pre-approved `token` from `from`.
/// Checks four wildcard combinations in order (most-specific first).
fn token_approved(store: &Store, to: &str, token: &str, from: &str) -> bool {
    store.state_get(&format!("token_approval:{}:{}:{}", to, token, from)).is_some()
        || store.state_get(&format!("token_approval:{}:*:{}", to, from)).is_some()
        || store.state_get(&format!("token_approval:{}:{}:*", to, token)).is_some()
        || store.state_get(&format!("token_approval:{}:*:*", to)).is_some()
}

enum SpamGate {
    /// On-chain fee: debit fee_token from sender, credit to recipient.
    OnChain { fee: u64, fee_token: String },
    /// Off-chain fee: sender must use SpamGatePayEvm with an EVM tx proof.
    Evm { evm_address: String, evm_chain_id: u64 },
}

/// Returns the gate config for an unsolicited `token` landing at `to`,
/// or None if no gate is set. Checks exact token first, then wildcard.
fn spam_gate(store: &Store, to: &str, token: &str) -> Option<SpamGate> {
    for key in [
        format!("spam_gate:{}:{}", to, token),
        format!("spam_gate:{}:*", to),
    ] {
        if let Some(raw) = store.state_get(&key) {
            if let Ok(j) = serde_json::from_slice::<serde_json::Value>(&raw) {
                // EVM gate takes priority when evm_address is present.
                if let (Some(addr), Some(chain_id)) = (
                    j["evm_address"].as_str().filter(|s| !s.is_empty()),
                    j["evm_chain_id"].as_u64(),
                ) {
                    return Some(SpamGate::Evm {
                        evm_address: addr.to_owned(),
                        evm_chain_id: chain_id,
                    });
                }
                if let Some(fee) = j["fee"].as_u64() {
                    let fee_token = j["fee_token"].as_str()
                        .unwrap_or(NATIVE_TOKEN).to_owned();
                    return Some(SpamGate::OnChain { fee, fee_token });
                }
            }
        }
    }
    None
}

pub struct Chain {
    pub store: Store,
    pub current_epoch: Arc<RwLock<u64>>,
    #[allow(dead_code)]
    pub node_id: String,
    pub chain_id: String,
    /// Serialises all write paths: nonce-check → debit/credit → nonce-bump.
    pub write_lock: parking_lot::Mutex<()>,
}

impl Chain {
    pub fn new(store: Store, node_id: String, chain_id: String) -> Self {
        // Prefer the persisted epoch counter over the latest block epoch, because
        // sealed epochs may advance past the last produced block (clock-only nodes).
        let persisted = store.get_meta("current_epoch")
            .and_then(|b| b.try_into().ok())
            .map(u64::from_le_bytes);
        let current_epoch = persisted.unwrap_or_else(|| store.latest_epoch().unwrap_or(0));
        Self {
            store,
            current_epoch: Arc::new(RwLock::new(current_epoch)),
            node_id,
            chain_id,
            write_lock: Mutex::new(()),
        }
    }

    pub fn current_epoch(&self) -> u64 {
        *self.current_epoch.read()
    }

    /// Apply a single ledger entry to state. Returns Ok(()) or a validation error.
    pub fn apply_entry(&self, entry: &LedgerEntry) -> Result<()> {
        match entry {
            LedgerEntry::GenesisAlloc { account, amount, token } => {
                self.ensure_account(account, 0)?;
                self.store.credit(account, token, *amount)?;
                info!("genesis alloc: {} {} → {}", amount, token, account);
            }

            LedgerEntry::AccountCreate { account, keys, epoch } => {
                if self.store.get_account(account)?.is_some() {
                    return Ok(()); // idempotent
                }
                let state = serde_json::json!({
                    "account_id": account,
                    "created_epoch": epoch,
                    "keys": keys,
                    "nonce": 0,
                    "stake": 0,
                });
                self.store.set_account(account, &state)?;
            }

            LedgerEntry::Transfer { from, to, amount, token, epoch, nonce, .. } => {
                anyhow::ensure!(*amount > 0, "transfer amount must be positive");
                self.ensure_account(to, *epoch)?;
                self.store.debit(from, token, *amount)?;
                // Native token or pre-approved type/sender: instant credit, no fee.
                if token == NATIVE_TOKEN || token_approved(&self.store, to, token, from) {
                    self.store.credit(to, token, *amount)?;
                } else if let Some(gate) = spam_gate(&self.store, to, token) {
                    match gate {
                        SpamGate::OnChain { fee, fee_token } => {
                            anyhow::ensure!(
                                self.store.get_balance(from, &fee_token) >= fee,
                                "sender '{}' needs {} {} to pass spam gate for '{}'", from, fee, fee_token, to
                            );
                            self.store.debit(from, &fee_token, fee)?;
                            self.store.credit(to, &fee_token, fee)?;
                            self.store.credit(to, token, *amount)?;
                        }
                        SpamGate::Evm { evm_address, evm_chain_id } => {
                            anyhow::bail!(
                                "recipient '{}' requires EVM payment to {} on chain {}; use SpamGatePayEvm",
                                to, evm_address, evm_chain_id
                            );
                        }
                    }
                } else {
                    // No approval, no gate: pending — recipient must TokenAccept.
                    // Expires 30 epochs (~15 min); sweep refunds sender on expiry.
                    let pending_key = format!("pending_transfer:{}:{}:{}:{}", to, token, from, nonce);
                    let val = serde_json::to_vec(&serde_json::json!({
                        "from": from, "to": to, "token": token,
                        "amount": amount, "expires_epoch": epoch + 30,
                    }))?;
                    self.store.state_set(&pending_key, &val)?;
                }
            }

            LedgerEntry::Stake { account, amount, .. } => {
                anyhow::ensure!(*amount > 0, "stake amount must be positive");
                self.store.debit(account, NATIVE_TOKEN, *amount)?;
                let current_stake = self.store.get_stake(account);
                let new_stake = current_stake.checked_add(*amount)
                    .ok_or_else(|| anyhow::anyhow!("stake overflow for '{}'", account))?;
                self.store.set_stake(account, new_stake)?;
            }

            LedgerEntry::Unstake { account, amount, .. } => {
                let current_stake = self.store.get_stake(account);
                anyhow::ensure!(current_stake >= *amount, "insufficient stake");
                self.store.set_stake(account, current_stake - amount)?;
                self.store.credit(account, NATIVE_TOKEN, *amount)?;
            }

            LedgerEntry::Mine { miner, epoch, model, input_tokens, output_tokens, tool_calls, hw_tier, compute_proof } => {
                self.ensure_account(miner, *epoch)?;
                let key = format!("mine:{}:{}", epoch, miner);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "miner": miner, "epoch": epoch,
                        "model": model, "input_tokens": input_tokens,
                        "output_tokens": output_tokens, "tool_calls": tool_calls,
                        "hw_tier": hw_tier, "compute_proof": compute_proof,
                    })).unwrap_or_default());
            }

            LedgerEntry::MineReward { miner, amount, epoch } => {
                self.ensure_account(miner, *epoch)?;
                if era(*epoch) >= RECYCLE_ERA {
                    // Era 5+: transfer from recycle fund rather than minting.
                    self.store.debit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, *amount)
                        .map_err(|e| anyhow::anyhow!("recycle fund insufficient for era-5 reward: {}", e))?;
                }
                self.store.credit(miner, NATIVE_TOKEN, *amount)?;
                info!("mine reward: {} dreams → {} (epoch {})", amount, miner, epoch);
            }

            LedgerEntry::EpochSeal { node_id: _, epoch, .. } => {
                let ep = *epoch as u64;
                let mut current = self.current_epoch.write();
                if ep > *current {
                    *current = ep;
                    // Persist so restarts recover the correct epoch even if no block was produced.
                    let _ = self.store.set_meta("current_epoch", &ep.to_le_bytes());
                }
            }

            LedgerEntry::EpochFinalize { epoch, rewards_hash, quorum, state_root, timestamp, .. } => {
                let meta = serde_json::json!({
                    "epoch": epoch,
                    "rewards_hash": rewards_hash,
                    "quorum": quorum,
                    "state_root": state_root,
                    "finalized_at": timestamp,
                    "finalized": true,
                });
                self.store.set_epoch_meta(*epoch, &meta)?;
            }

            LedgerEntry::AccountUpdateKey { account, role, new_public_key, epoch, .. } => {
                // Create the account if it doesn't exist yet (first-time key registration).
                self.ensure_account(account, *epoch)?;
                let mut state = self.store.get_account(account)?
                    .unwrap_or_else(|| serde_json::json!({ "nonce": 0 }));
                if state.get("keys").is_none() || !state["keys"].is_object() {
                    state["keys"] = serde_json::json!({});
                }
                state["keys"][role] = serde_json::json!(new_public_key);
                self.store.set_account(account, &state)?;
                info!("key '{}' registered for account '{}'", role, account);
            }

            LedgerEntry::SensorReading { .. }
            | LedgerEntry::BlobStore { .. }
            | LedgerEntry::ContractDeploy { .. }
            | LedgerEntry::ContractCall { .. } => {
                // Accepted, no balance mutations needed at base layer
            }

            // Track storage heartbeats so the clock can compute storage rewards at seal time.
            LedgerEntry::StorageHeartbeat { node_id, epoch, bytes_proven, query_count, .. } => {
                let key = format!("storage_beat:{}:{}", epoch, node_id);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "node_id": node_id, "epoch": epoch,
                        "bytes_proven": bytes_proven, "query_count": query_count,
                    })).unwrap_or_default());
            }

            // Track sensor commits per sensor_id (not per owner) so reward scoring
            // can apply type-aware sensor_score() per individual sensor.
            LedgerEntry::SensorDataCommit { sensor_id, owner, epoch, reading_count, sensor_type, .. } => {
                let key = format!("sensor_commit:{}:{}", epoch, sensor_id);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "sensor_id": sensor_id, "owner": owner, "epoch": epoch,
                        "reading_count": reading_count, "sensor_type": sensor_type,
                    })).unwrap_or_default());
            }

            // Freeport commerce — recorded on-chain, state managed by btcpc-market sidecar
            LedgerEntry::StoreUpdate { .. }
            | LedgerEntry::ProductCreate { .. }
            | LedgerEntry::ProductUpdate { .. }
            | LedgerEntry::OrderPlace { .. }
            | LedgerEntry::OrderFulfill { .. }
            | LedgerEntry::OrderCancel { .. }
            | LedgerEntry::OrderDispute { .. }
            | LedgerEntry::EscrowRelease { .. }
            | LedgerEntry::FlashSale { .. }
            // Verasens sensors — recorded on-chain, state in sidecar
            | LedgerEntry::SensorRegister { .. }
            | LedgerEntry::SensorKeyRegister { .. }
            | LedgerEntry::SensorVouch { .. }
            | LedgerEntry::DeviceKeyRegister { .. }
            | LedgerEntry::GatewayHeartbeat { .. } => {
                // Recorded in the ledger; state is managed by protocol sidecars.
            }

            // Device yield stake — tracked on-chain so overbid handler can distribute premium.
            LedgerEntry::DeviceYieldStake { device_serial, staker, amount, epoch, .. } => {
                self.ensure_account(staker, *epoch)?;
                self.store.debit(staker, NATIVE_TOKEN, *amount)?;
                let key = format!("yield_stake:{}:{}", device_serial, staker);
                let prev: u64 = self.store.state_get(&key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["amount"].as_u64())
                    .unwrap_or(0);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "device_serial": device_serial, "staker": staker,
                        "amount": prev + amount,
                    })).unwrap_or_default());
            }

            LedgerEntry::DeviceYieldUnstake { device_serial, staker, amount, .. } => {
                let key = format!("yield_stake:{}:{}", device_serial, staker);
                let current: u64 = self.store.state_get(&key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["amount"].as_u64())
                    .unwrap_or(0);
                anyhow::ensure!(current >= *amount, "insufficient yield stake to unstake");
                let remaining = current - amount;
                if remaining == 0 {
                    self.store.state_delete(&key)?;
                } else {
                    let _ = self.store.state_set(&key,
                        &serde_json::to_vec(&serde_json::json!({
                            "device_serial": device_serial, "staker": staker,
                            "amount": remaining,
                        })).unwrap_or_default());
                }
                self.store.credit(staker, NATIVE_TOKEN, *amount)?;
            }

            // ── LinkGit: core chain state (repo registry, refs, access grants) ──
            LedgerEntry::LinkGitRepoCreate { repo_id, owner, name, visibility, hide_key, epoch, .. } => {
                let repo = serde_json::json!({
                    "repo_id": repo_id,
                    "owner": owner,
                    "name": name,
                    "visibility": visibility,
                    "hide_key": hide_key,
                    "refs": {},
                    "created_epoch": epoch,
                });
                self.store.set_meta(
                    &format!("linkgit:repo:{}", repo_id),
                    repo.to_string().as_bytes(),
                )?;
            }
            LedgerEntry::LinkGitRefUpdate { repo_id, ref_name, commit_hash, .. } => {
                let key = format!("linkgit:repo:{}", repo_id);
                if let Some(bytes) = self.store.get_meta(&key) {
                    if let Ok(mut repo) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        repo["refs"][ref_name] = serde_json::json!(commit_hash);
                        self.store.set_meta(&key, repo.to_string().as_bytes())?;
                    }
                }
            }
            LedgerEntry::LinkGitAccessGrant { repo_id, grantee, encrypted_key, .. } => {
                let key = format!("linkgit:access:{}:{}", repo_id, grantee);
                self.store.set_meta(&key, encrypted_key.as_bytes())?;
            }
            LedgerEntry::LinkGitAccessRevoke { repo_id, grantee, .. } => {
                let key = format!("linkgit:access:{}:{}", repo_id, grantee);
                self.store.state_delete(&key)?;
            }
            // Recorded in ledger; no core state change needed
            LedgerEntry::LinkGitPruneProof { .. }
            | LedgerEntry::LinkGitStorageExtend { .. } => {}

            // ── Inference marketplace ─────────────────────────────────────────
            LedgerEntry::InferenceJobPost { .. } => {
                inference::apply_post(self, entry)?;
            }
            LedgerEntry::InferenceJobBid { .. } => {
                inference::apply_bid(self, entry)?;
            }
            LedgerEntry::InferenceJobAward { .. } => {
                inference::apply_award(self, entry)?;
            }
            LedgerEntry::InferenceJobComplete { .. } => {
                inference::apply_complete(self, entry)?;
            }
            LedgerEntry::InferenceJobVerify { .. } => {
                inference::apply_verify(self, entry)?;
            }
            LedgerEntry::InferenceJobClaim { .. } => {
                inference::apply_claim(self, entry)?;
            }
            LedgerEntry::InferenceReviewVote { .. } => {
                inference::apply_review_vote(self, entry)?;
            }
            LedgerEntry::InferenceJobPay { .. } => {
                inference::apply_pay(self, entry)?;
            }
            LedgerEntry::InferenceJobCancel { .. } => {
                inference::apply_cancel(self, entry)?;
            }

            // ── Clock reward ──────────────────────────────────────────────────
            LedgerEntry::ClockReward { node_id, amount, epoch } => {
                self.ensure_account(node_id, *epoch)?;
                let _ = CLOCK_REWARD_DREAMS; // used by caller to size per-node amounts
                self.store.credit(node_id, NATIVE_TOKEN, *amount)?;
            }

            // ── Testnet ────────────────────────────────────────────────────────
            LedgerEntry::TestnetOperatorRegister { mainnet_account, testnet_node_id, testnet_chain_id, epoch, .. } => {
                self.ensure_account(mainnet_account, *epoch)?;
                let key = format!("testnet_op:{}:{}", mainnet_account, testnet_chain_id);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "mainnet_account": mainnet_account,
                        "testnet_node_id": testnet_node_id,
                        "testnet_chain_id": testnet_chain_id,
                        "registered_epoch": epoch,
                    })).unwrap_or_default());
            }
            LedgerEntry::TestnetReward { mainnet_account, amount, epoch, .. } => {
                self.ensure_account(mainnet_account, *epoch)?;
                // Drain from testnet fund — fund was seeded at genesis.
                let _ = self.store.debit(TESTNET_FUND_ACCOUNT, NATIVE_TOKEN, *amount);
                self.store.credit(mainnet_account, NATIVE_TOKEN, *amount)?;
            }

            // ── Layer B epoch rewards ─────────────────────────────────────────
            LedgerEntry::StorageReward { node_id, amount, epoch } => {
                self.ensure_account(node_id, *epoch)?;
                self.store.credit(node_id, NATIVE_TOKEN, *amount)?;
            }
            LedgerEntry::SensorReward { node_id, amount, epoch } => {
                self.ensure_account(node_id, *epoch)?;
                self.store.credit(node_id, NATIVE_TOKEN, *amount)?;
            }
            LedgerEntry::VerifierReward { node_id, amount, epoch } => {
                self.ensure_account(node_id, *epoch)?;
                self.store.credit(node_id, NATIVE_TOKEN, *amount)?;
            }
            LedgerEntry::ServiceReward { node_id, amount, epoch } => {
                self.ensure_account(node_id, *epoch)?;
                self.store.credit(node_id, NATIVE_TOKEN, *amount)?;
            }

            // ── Service heartbeat ─────────────────────────────────────────────
            LedgerEntry::ServiceHeartbeat { node_id, epoch, container_hours, .. } => {
                let key = format!("service_beat:{}:{}", epoch, node_id);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "node_id": node_id, "epoch": epoch,
                        "container_hours": container_hours,
                    })).unwrap_or_default());
            }

            // ── Verifier claim — worker will encrypt payload to verifier's memo key ──
            LedgerEntry::InferenceVerifyClaim { job_id, verifier, epoch, .. } => {
                let key = format!("verify_claim:{}:{}", job_id, verifier);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "job_id": job_id, "verifier": verifier, "epoch": epoch,
                    })).unwrap_or_default());
            }

            // ── Sensor data purchase ──────────────────────────────────────────
            LedgerEntry::SensorDataPurchase { buyer, owner, fee, epoch, .. } => {
                // Split fee: 80% to sensor owner, 15% to storage contract, 5% to recycle
                let owner_share   = (*fee as u128 * 80 / 100) as u64;
                let recycle_share = fee.saturating_sub(owner_share);
                self.store.debit(buyer, NATIVE_TOKEN, *fee)?;
                self.ensure_account(owner, *epoch)?;
                self.store.credit(owner, NATIVE_TOKEN, owner_share)?;
                if recycle_share > 0 {
                    let _ = self.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_share);
                }
            }

            // ── Mempool rewards ───────────────────────────────────────────────
            LedgerEntry::MempoolReward { operator, amount, epoch } => {
                self.ensure_account(operator, *epoch)?;
                self.store.credit(operator, NATIVE_TOKEN, *amount)?;
            }

            // ── Mempool operator registration ─────────────────────────────────
            LedgerEntry::MempoolOperatorRegister { operator, amount, epoch, .. } => {
                self.ensure_account(operator, *epoch)?;
                self.store.debit(operator, NATIVE_TOKEN, *amount)?;
                // Record operator registration with staked amount
                let key = format!("mempool_op:{}", operator);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "operator": operator, "stake": amount, "registered_epoch": epoch,
                    })).unwrap_or_default());
            }

            // ── Mempool heartbeat ─────────────────────────────────────────────
            LedgerEntry::MempoolHeartbeat { operator, epoch, propagation_latency_ms, entries_relayed, .. } => {
                let key = format!("mempool_beat:{}:{}", epoch, operator);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "operator": operator, "epoch": epoch,
                        "latency_ms": propagation_latency_ms,
                        "entries_relayed": entries_relayed,
                    })).unwrap_or_default());
            }

            // ── Device claim stake ────────────────────────────────────────────
            LedgerEntry::DeviceClaimStake { device_serial, owner, amount, epoch, .. } => {
                self.ensure_account(owner, *epoch)?;
                let claim_key = format!("device_claim:{}", device_serial);

                if let Some(existing) = self.store.state_get(&claim_key) {
                    if let Ok(j) = serde_json::from_slice::<serde_json::Value>(&existing) {
                        let existing_owner = j["owner"].as_str().unwrap_or("");
                        let existing_stake = j["stake"].as_u64().unwrap_or(0);

                        if !existing_owner.is_empty() && existing_owner != owner.as_str() {
                            // Overbid path: new stake must be ≥ old_stake × 1.5
                            let required = ((existing_stake as u128)
                                .saturating_mul(DEVICE_CLAIM_OVERBID_NUM)
                                / DEVICE_CLAIM_OVERBID_DENOM) as u64;
                            anyhow::ensure!(
                                *amount >= required,
                                "device '{}' claimed by '{}' (stake {}); overbid requires ≥ {} (1.5×)",
                                device_serial, existing_owner, existing_stake, required
                            );

                            // Return old claim owner's full principal
                            self.store.credit(existing_owner, NATIVE_TOKEN, existing_stake)?;

                            // Distribute overbid premium to yield stakers
                            let premium = amount.saturating_sub(existing_stake);
                            if premium > 0 {
                                let staker_share = (premium as u128 * OVERCLAIM_STAKER_SHARE_BPS as u128 / 10_000) as u64;
                                let recycle_share = premium.saturating_sub(staker_share);

                                // Collect all yield stakers for this device
                                let yield_key_prefix = format!("yield_stake:{}:", device_serial);
                                let stakers: Vec<(String, u64)> = self.store.state_scan_prefix(&yield_key_prefix)
                                    .into_iter()
                                    .filter_map(|(_, v)| {
                                        let js = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
                                        let s = js["staker"].as_str()?.to_owned();
                                        let a = js["amount"].as_u64().unwrap_or(0);
                                        if a > 0 { Some((s, a)) } else { None }
                                    }).collect();

                                let total_yield_stake: u64 = stakers.iter().map(|(_, a)| a).sum();
                                if total_yield_stake > 0 && staker_share > 0 {
                                    for (staker, stake_amount) in &stakers {
                                        let payout = (staker_share as u128 * *stake_amount as u128
                                            / total_yield_stake as u128) as u64;
                                        if payout > 0 {
                                            self.store.credit(staker, NATIVE_TOKEN, payout)?;
                                        }
                                    }
                                } else {
                                    // No yield stakers — full premium to recycle
                                    let _ = self.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, staker_share);
                                }
                                if recycle_share > 0 {
                                    let _ = self.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_share);
                                }
                            }

                            info!(
                                "device claim: '{}' overbid by '{}' (old_stake={} new_stake={} premium={}), \
                                 '{}' stake returned",
                                device_serial, owner, existing_stake, amount,
                                amount.saturating_sub(existing_stake), existing_owner
                            );
                        }
                    }
                }

                self.store.debit(owner, NATIVE_TOKEN, *amount)?;
                let _ = self.store.state_set(&claim_key,
                    &serde_json::to_vec(&serde_json::json!({
                        "device_serial": device_serial, "owner": owner,
                        "stake": amount, "claimed_epoch": epoch,
                    })).unwrap_or_default());
            }

            LedgerEntry::DeviceClaimUnstake { device_serial, owner, epoch, .. } => {
                let claim_key = format!("device_claim:{}", device_serial);
                let stake_amount: u64 = self.store.state_get(&claim_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| {
                        if j["owner"].as_str() == Some(owner.as_str()) {
                            j["stake"].as_u64()
                        } else { None }
                    })
                    .ok_or_else(|| anyhow::anyhow!("no active claim for serial '{}' by '{}'", device_serial, owner))?;
                self.store.state_delete(&claim_key)?;
                self.ensure_account(owner, *epoch)?;
                self.store.credit(owner, NATIVE_TOKEN, stake_amount)?;
            }

            // ── Scoped Delegation ─────────────────────────────────────────────
            LedgerEntry::DelegationGrant { from, to, capabilities, expires_epoch, .. } => {
                let key = format!("delegation:{}:{}", from, to);
                let val = serde_json::to_vec(&serde_json::json!({
                    "from": from, "to": to,
                    "capabilities": capabilities,
                    "expires_epoch": expires_epoch,
                }))?;
                self.store.state_set(&key, &val)?;
            }
            LedgerEntry::DelegationRevoke { from, to, .. } => {
                self.store.state_delete(&format!("delegation:{}:{}", from, to))?;
            }

            // ── Device Yield Opt-In ───────────────────────────────────────────
            LedgerEntry::DeviceYieldOptIn { device_serial, owner, max_stakers, .. } => {
                let claim_key = format!("device_claim:{}", device_serial);
                let current_owner = self.store.state_get(&claim_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["owner"].as_str().map(str::to_owned));
                anyhow::ensure!(current_owner.as_deref() == Some(owner.as_str()),
                    "only the device owner can opt in to yield sharing");
                let val = serde_json::to_vec(&serde_json::json!({
                    "owner": owner, "max_stakers": max_stakers, "opted_in": true,
                }))?;
                self.store.state_set(&format!("yield_config:{}", device_serial), &val)?;
            }
            LedgerEntry::DeviceYieldOptOut { device_serial, owner, .. } => {
                let config_key = format!("yield_config:{}", device_serial);
                let stored_owner = self.store.state_get(&config_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["owner"].as_str().map(str::to_owned));
                anyhow::ensure!(stored_owner.as_deref() == Some(owner.as_str()),
                    "only the device owner can opt out of yield sharing");
                self.store.state_delete(&config_key)?;
            }

            // ── Permissive Token Model ────────────────────────────────────────
            LedgerEntry::SpamGateSet { account, token, fee, fee_token, evm_address, evm_chain_id, .. } => {
                let key = format!("spam_gate:{}:{}", account, token);
                let val = serde_json::to_vec(&serde_json::json!({
                    "fee": fee,
                    "fee_token": fee_token,
                    "evm_address": evm_address,
                    "evm_chain_id": evm_chain_id,
                }))?;
                self.store.state_set(&key, &val)?;
            }
            LedgerEntry::SpamGatePayEvm { from, to, token, amount, evm_tx_hash, evm_chain_id, epoch, nonce, .. } => {
                // Verify an EVM gate exists for this recipient+token.
                let gate_ok = matches!(
                    spam_gate(&self.store, to, token),
                    Some(SpamGate::Evm { evm_chain_id: cid, .. }) if cid == *evm_chain_id
                );
                anyhow::ensure!(gate_ok,
                    "no EVM spam gate on chain {} for '{}' receiving '{}'", evm_chain_id, to, token);
                // Debit sender's token balance.
                self.store.debit(from, token, *amount)?;
                // Create pending record with EVM proof attached — recipient inspects and TokenAccepts.
                let pending_key = format!("pending_transfer:{}:{}:{}:{}", to, token, from, nonce);
                let val = serde_json::to_vec(&serde_json::json!({
                    "from": from, "to": to, "token": token, "amount": amount,
                    "expires_epoch": epoch + 30,
                    "evm_tx_hash": evm_tx_hash,
                    "evm_chain_id": evm_chain_id,
                }))?;
                self.store.state_set(&pending_key, &val)?;
            }
            LedgerEntry::SpamGateClear { account, token, .. } => {
                self.store.state_delete(&format!("spam_gate:{}:{}", account, token))?;
            }
            LedgerEntry::TokenApprove { account, token, from, .. } => {
                let key = format!("token_approval:{}:{}:{}", account, token, from);
                self.store.state_set(&key, b"1")?;
            }
            LedgerEntry::TokenRevoke { account, token, from, .. } => {
                let key = format!("token_approval:{}:{}:{}", account, token, from);
                self.store.state_delete(&key)?;
            }
            LedgerEntry::TokenAccept { to, token, from, pending_nonce, .. } => {
                let pending_key = format!("pending_transfer:{}:{}:{}:{}", to, token, from, pending_nonce);
                let amount: u64 = self.store.state_get(&pending_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["amount"].as_u64())
                    .ok_or_else(|| anyhow::anyhow!("no pending transfer found"))?;
                self.store.state_delete(&pending_key)?;
                self.store.credit(to, token, amount)?;
            }
            LedgerEntry::TokenReject { to, token, from, pending_nonce, .. } => {
                let pending_key = format!("pending_transfer:{}:{}:{}:{}", to, token, from, pending_nonce);
                let amount: u64 = self.store.state_get(&pending_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["amount"].as_u64())
                    .ok_or_else(|| anyhow::anyhow!("no pending transfer found"))?;
                self.store.state_delete(&pending_key)?;
                self.store.credit(from, token, amount)?;
            }

            // ── Wallet Family ─────────────────────────────────────────────────
            LedgerEntry::WalletFamilyPublish { account, chains, .. }
            | LedgerEntry::WalletFamilyAdd { account, chains, .. } => {
                let family_key = format!("wallet_family:{}", account);
                // Load existing list (additive-only — never remove).
                let mut existing: Vec<serde_json::Value> = self.store.state_get(&family_key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default();
                for ca in chains {
                    let entry_json = serde_json::json!({
                        "chain": ca.chain,
                        "address": ca.address,
                        "derivation_path": ca.derivation_path,
                    });
                    // Dedup: skip if (chain, address) already present.
                    let already = existing.iter().any(|e| {
                        e["chain"].as_str() == Some(&ca.chain)
                            && e["address"].as_str() == Some(&ca.address)
                    });
                    if !already {
                        // Reverse index: wallet_addr:{chain}:{address} → account
                        let rev_key = format!("wallet_addr:{}:{}", ca.chain, ca.address);
                        self.store.state_set(&rev_key, account.as_bytes())?;
                        existing.push(entry_json);
                    }
                }
                let val = serde_json::to_vec(&existing)?;
                self.store.state_set(&family_key, &val)?;
            }

        }

        Ok(())
    }

    /// Apply a batch of entries from a block payload.
    pub fn apply_block_entries(&self, entries: &[LedgerEntry]) -> usize {
        let mut applied = 0;
        for entry in entries {
            match self.apply_entry(entry) {
                Ok(_) => applied += 1,
                Err(e) => warn!("entry rejected: {}: {:?}", e, entry),
            }
        }
        applied
    }

    pub fn get_balance(&self, account: &str, token: &str) -> u64 {
        self.store.get_balance(account, token)
    }

    pub fn get_stake(&self, account: &str) -> u64 {
        self.store.get_stake(account)
    }

    fn ensure_account(&self, account: &AccountId, epoch: u64) -> Result<()> {
        if self.store.get_account(account)?.is_none() {
            self.store.set_account(account, &serde_json::json!({
                "account_id": account,
                "created_epoch": epoch,
                "nonce": 0,
                "stake": 0,
            }))?;
        }
        Ok(())
    }
}

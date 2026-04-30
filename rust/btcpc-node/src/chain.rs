//! Chain state machine — applies ledger entries and advances chain state.

use std::sync::Arc;
use anyhow::Result;
use parking_lot::{Mutex, RwLock};
use tracing::{info, warn};
use btcpc_types::{AccountId, LedgerEntry, NATIVE_TOKEN, CLOCK_REWARD_DREAMS, era, RECYCLE_ERA, RECYCLE_FUND_ACCOUNT};

use crate::inference;
use crate::store::Store;

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

            LedgerEntry::Transfer { from, to, amount, token, epoch, nonce: _, .. } => {
                anyhow::ensure!(*amount > 0, "transfer amount must be positive");
                self.ensure_account(to, *epoch)?;
                self.store.debit(from, token, *amount)?;
                self.store.credit(to, token, *amount)?;
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

            LedgerEntry::EpochFinalize { epoch, state_root, timestamp, .. } => {
                let meta = serde_json::json!({
                    "epoch": epoch,
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
            LedgerEntry::StorageHeartbeat { node_id, epoch, bytes_proven, .. } => {
                let key = format!("storage_beat:{}:{}", epoch, node_id);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "node_id": node_id, "epoch": epoch, "bytes_proven": bytes_proven,
                    })).unwrap_or_default());
            }

            // Track sensor data commits so the clock can compute sensor rewards.
            LedgerEntry::SensorDataCommit { owner, epoch, reading_count, .. } => {
                let key = format!("sensor_commit:{}:{}", epoch, owner);
                // Accumulate reading_count in case multiple sensors from the same owner commit.
                let prev: u64 = self.store.state_get(&key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["reading_count"].as_u64())
                    .unwrap_or(0);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "owner": owner, "epoch": epoch,
                        "reading_count": prev + reading_count,
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
            | LedgerEntry::DeviceYieldStake { .. }
            | LedgerEntry::DeviceYieldUnstake { .. }
            | LedgerEntry::GatewayHeartbeat { .. } => {
                // Recorded in the ledger; state is managed by protocol sidecars.
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

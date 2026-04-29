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

            LedgerEntry::AccountCreate { account, public_key, epoch } => {
                if self.store.get_account(account)?.is_some() {
                    return Ok(()); // idempotent
                }
                let state = serde_json::json!({
                    "account_id": account,
                    "created_epoch": epoch,
                    "public_key": public_key,
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

            LedgerEntry::Mine { miner, epoch, .. } => {
                self.ensure_account(miner, *epoch)?;
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

            LedgerEntry::AccountUpdateKey { account, new_public_key, epoch, .. } => {
                // Create the account if it doesn't exist yet (first-time key registration).
                self.ensure_account(account, *epoch)?;
                let mut state = self.store.get_account(account)?
                    .unwrap_or_else(|| serde_json::json!({ "nonce": 0 }));
                state["public_key"] = serde_json::json!(new_public_key);
                self.store.set_account(account, &state)?;
                info!("key registered for account '{}'", account);
            }

            LedgerEntry::SensorReading { .. }
            | LedgerEntry::BlobStore { .. }
            | LedgerEntry::ContractDeploy { .. }
            | LedgerEntry::ContractCall { .. } => {
                // Accepted, no balance mutations needed at base layer
            }

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

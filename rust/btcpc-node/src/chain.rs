//! Chain state machine — applies ledger entries and advances chain state.

use std::sync::Arc;
use anyhow::Result;
use parking_lot::RwLock;
use tracing::{info, warn};
use btcpc_types::{AccountId, LedgerEntry, NATIVE_TOKEN};

use crate::store::Store;

pub struct Chain {
    pub store: Store,
    pub current_epoch: Arc<RwLock<u64>>,
    #[allow(dead_code)]
    pub node_id: String,
}

impl Chain {
    pub fn new(store: Store, node_id: String) -> Self {
        let current_epoch = store.latest_epoch().unwrap_or(0) as u64;
        Self {
            store,
            current_epoch: Arc::new(RwLock::new(current_epoch)),
            node_id,
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
                self.store.credit(miner, NATIVE_TOKEN, *amount)?;
                info!("mine reward: {} BTCPC → {} (epoch {})", amount, miner, epoch);
            }

            LedgerEntry::EpochSeal { node_id: _, epoch, .. } => {
                let mut current = self.current_epoch.write();
                if *epoch as u64 > *current {
                    *current = *epoch as u64;
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

            LedgerEntry::SensorReading { .. }
            | LedgerEntry::BlobStore { .. }
            | LedgerEntry::InferenceJob { .. }
            | LedgerEntry::ContractDeploy { .. }
            | LedgerEntry::ContractCall { .. }
            | LedgerEntry::AccountUpdateKey { .. } => {
                // Accepted, no balance mutations needed at base layer
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

//! Lightweight chain state for the Android micronode.
//! Applies LedgerEntries to the sled store and tracks epoch progress.

use std::sync::Arc;
use anyhow::{bail, Result};
use parking_lot::RwLock;
use tracing::warn;

use btcpc_types::{
    LedgerEntry, NATIVE_TOKEN,
    block_reward_at, era, RECYCLE_ERA, RECYCLE_REWARD_RATE, RECYCLE_REWARD_DENOM,
    RECYCLE_FUND_ACCOUNT,
};

use crate::store::Store;

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
            Mine { miner, epoch, .. } => {
                // Record that this miner produced a block this epoch.
                let key = format!("mine:{}:{}", epoch, miner);
                let _ = self.store.set_meta(&key, b"1");
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
            InferenceJobPay { worker, worker_amount, .. } => {
                self.store.credit(worker, NATIVE_TOKEN, *worker_amount);
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

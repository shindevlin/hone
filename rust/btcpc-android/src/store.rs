//! Sled-backed state store — drop-in replacement for the RocksDB store used on desktop.
//! Pure Rust, zero C++ build dependencies, works on Android arm64 out of the box.

use anyhow::Result;
use sled::{Db, Tree};

const TREE_BLOCKS:  &str = "blocks";
const TREE_STATE:   &str = "state";
const TREE_META:    &str = "meta";

#[derive(Clone)]
pub struct Store {
    db: Db,
}

impl Store {
    pub fn open(path: &str) -> Result<Self> {
        let db = sled::open(path)?;
        Ok(Store { db })
    }

    // ── Blocks ────────────────────────────────────────────────────────────────

    fn blocks(&self) -> Tree { self.db.open_tree(TREE_BLOCKS).expect("blocks tree") }

    pub fn write_block(&self, epoch: u64, data: &[u8]) -> Result<()> {
        self.blocks().insert(epoch.to_be_bytes(), data)?;
        Ok(())
    }

    pub fn read_block(&self, epoch: u64) -> Result<Option<Vec<u8>>> {
        Ok(self.blocks().get(epoch.to_be_bytes())?.map(|v| v.to_vec()))
    }

    pub fn has_block(&self, epoch: u64) -> bool {
        self.blocks().contains_key(epoch.to_be_bytes()).unwrap_or(false)
    }

    pub fn latest_epoch(&self) -> Option<u64> {
        let tree = self.blocks();
        let (k, _) = tree.last().ok()??;
        if k.len() == 8 {
            Some(u64::from_be_bytes(k[..8].try_into().ok()?))
        } else {
            None
        }
    }

    // ── Balances ──────────────────────────────────────────────────────────────

    fn state(&self) -> Tree { self.db.open_tree(TREE_STATE).expect("state tree") }

    fn balance_key(account: &str, token: &str) -> Vec<u8> {
        format!("bal:{}:{}", account, token).into_bytes()
    }

    pub fn get_balance(&self, account: &str, token: &str) -> u64 {
        let key = Self::balance_key(account, token);
        self.state().get(&key).ok().flatten()
            .and_then(|v| v[..].try_into().ok().map(u64::from_be_bytes))
            .unwrap_or(0)
    }

    pub fn set_balance(&self, account: &str, token: &str, amount: u64) {
        let key = Self::balance_key(account, token);
        let _ = self.state().insert(key, &amount.to_be_bytes());
    }

    pub fn credit(&self, account: &str, token: &str, amount: u64) {
        let cur = self.get_balance(account, token);
        self.set_balance(account, token, cur.saturating_add(amount));
    }

    pub fn debit(&self, account: &str, token: &str, amount: u64) -> bool {
        let cur = self.get_balance(account, token);
        if cur < amount { return false; }
        self.set_balance(account, token, cur - amount);
        true
    }

    // ── Account nonce ─────────────────────────────────────────────────────────

    pub fn get_nonce(&self, account: &str) -> u64 {
        let key = format!("nonce:{}", account).into_bytes();
        self.state().get(&key).ok().flatten()
            .and_then(|v| v[..].try_into().ok().map(u64::from_be_bytes))
            .unwrap_or(0)
    }

    pub fn increment_nonce(&self, account: &str) -> u64 {
        let next = self.get_nonce(account) + 1;
        let key = format!("nonce:{}", account).into_bytes();
        let _ = self.state().insert(key, &next.to_be_bytes());
        next
    }

    // ── Stakes ───────────────────────────────────────────────────────────────

    pub fn get_stake(&self, account: &str) -> u64 {
        let key = format!("stake:{}", account).into_bytes();
        self.state().get(&key).ok().flatten()
            .and_then(|v| v[..].try_into().ok().map(u64::from_be_bytes))
            .unwrap_or(0)
    }

    pub fn set_stake(&self, account: &str, amount: u64) {
        let key = format!("stake:{}", account).into_bytes();
        let _ = self.state().insert(key, &amount.to_be_bytes());
    }

    // ── Generic meta / state ──────────────────────────────────────────────────

    fn meta(&self) -> Tree { self.db.open_tree(TREE_META).expect("meta tree") }

    pub fn get_meta(&self, key: &str) -> Option<Vec<u8>> {
        self.meta().get(key.as_bytes()).ok().flatten().map(|v| v.to_vec())
    }

    pub fn set_meta(&self, key: &str, value: &[u8]) -> Result<()> {
        self.meta().insert(key.as_bytes(), value)?;
        Ok(())
    }

    pub fn get_json<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let bytes = self.state().get(key.as_bytes()).ok()??;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn set_json<T: serde::Serialize>(&self, key: &str, val: &T) -> Result<()> {
        let bytes = serde_json::to_vec(val)?;
        self.state().insert(key.as_bytes(), bytes.as_slice())?;
        Ok(())
    }

    pub fn delete_state(&self, key: &str) {
        let _ = self.state().remove(key.as_bytes());
    }

    pub fn scan_prefix(&self, prefix: &str) -> Vec<(String, Vec<u8>)> {
        self.state()
            .scan_prefix(prefix.as_bytes())
            .filter_map(|r| r.ok())
            .map(|(k, v)| (String::from_utf8_lossy(&k).into_owned(), v.to_vec()))
            .collect()
    }
}

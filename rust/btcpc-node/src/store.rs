//! RocksDB state store — single database, multiple column families.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;
use anyhow::{Context, Result};
use rocksdb::{DB, ColumnFamilyDescriptor, Options, WriteBatch, IteratorMode, Direction};

pub const CF_BLOCKS: &str = "blocks";
pub const CF_FINALITY: &str = "finality";
pub const CF_ACCOUNTS: &str = "accounts";
pub const CF_BALANCES: &str = "balances";
pub const CF_STAKES: &str = "stakes";
pub const CF_EPOCHS: &str = "epochs";
pub const CF_META: &str = "meta";

#[derive(Clone)]
pub struct Store {
    db: Arc<DB>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path)?;
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);

        let cfs = [CF_BLOCKS, CF_FINALITY, CF_ACCOUNTS, CF_BALANCES, CF_STAKES, CF_EPOCHS, CF_META]
            .iter().map(|name| ColumnFamilyDescriptor::new(*name, Options::default()))
            .collect::<Vec<_>>();

        let db = DB::open_cf_descriptors(&opts, path, cfs)
            .with_context(|| format!("opening RocksDB at {:?}", path))?;

        Ok(Self { db: Arc::new(db) })
    }

    // ── Blocks ───────────────────────────────────────────────────────────────

    pub fn write_block(&self, epoch: u32, data: &[u8]) -> Result<()> {
        let cf = self.db.cf_handle(CF_BLOCKS).context("blocks CF")?;
        self.db.put_cf(&cf, epoch.to_le_bytes(), data)?;
        Ok(())
    }

    pub fn read_block(&self, epoch: u32) -> Result<Option<Vec<u8>>> {
        let cf = self.db.cf_handle(CF_BLOCKS).context("blocks CF")?;
        Ok(self.db.get_cf(&cf, epoch.to_le_bytes())?)
    }

    pub fn has_block(&self, epoch: u32) -> bool {
        let Some(cf) = self.db.cf_handle(CF_BLOCKS) else { return false };
        self.db.get_cf(&cf, epoch.to_le_bytes()).map(|v| v.is_some()).unwrap_or(false)
    }

    pub fn latest_epoch(&self) -> Option<u32> {
        let cf = self.db.cf_handle(CF_BLOCKS)?;
        let mut iter = self.db.iterator_cf(&cf, IteratorMode::End);
        let (k, _) = iter.next()?.ok()?;
        Some(u32::from_le_bytes(k[..4].try_into().ok()?))
    }

    pub fn write_finality(&self, epoch: u32, data: &[u8]) -> Result<()> {
        let cf = self.db.cf_handle(CF_FINALITY).context("finality CF")?;
        self.db.put_cf(&cf, epoch.to_le_bytes(), data)?;
        Ok(())
    }

    pub fn read_finality(&self, epoch: u32) -> Result<Option<Vec<u8>>> {
        let cf = self.db.cf_handle(CF_FINALITY).context("finality CF")?;
        Ok(self.db.get_cf(&cf, epoch.to_le_bytes())?)
    }

    pub fn latest_finality(&self) -> Option<u32> {
        let cf = self.db.cf_handle(CF_FINALITY)?;
        let mut iter = self.db.iterator_cf(&cf, IteratorMode::End);
        let (k, _) = iter.next()?.ok()?;
        Some(u32::from_le_bytes(k[..4].try_into().ok()?))
    }

    // ── Accounts ─────────────────────────────────────────────────────────────

    pub fn get_account(&self, account_id: &str) -> Result<Option<serde_json::Value>> {
        let cf = self.db.cf_handle(CF_ACCOUNTS).context("accounts CF")?;
        match self.db.get_cf(&cf, account_id.as_bytes())? {
            Some(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
            None => Ok(None),
        }
    }

    pub fn set_account(&self, account_id: &str, data: &serde_json::Value) -> Result<()> {
        let cf = self.db.cf_handle(CF_ACCOUNTS).context("accounts CF")?;
        self.db.put_cf(&cf, account_id.as_bytes(), serde_json::to_vec(data)?)?;
        Ok(())
    }

    // ── Balances ─────────────────────────────────────────────────────────────
    // Key format: "account\0token" → u64 LE dreams

    pub fn get_balance(&self, account: &str, token: &str) -> u64 {
        let Some(cf) = self.db.cf_handle(CF_BALANCES) else { return 0 };
        let key = balance_key(account, token);
        self.db.get_cf(&cf, &key)
            .ok().flatten()
            .and_then(|b| b.try_into().ok())
            .map(u64::from_le_bytes)
            .unwrap_or(0)
    }

    pub fn set_balance(&self, account: &str, token: &str, amount: u64) -> Result<()> {
        let cf = self.db.cf_handle(CF_BALANCES).context("balances CF")?;
        self.db.put_cf(&cf, balance_key(account, token), amount.to_le_bytes())?;
        Ok(())
    }

    pub fn credit(&self, account: &str, token: &str, amount: u64) -> Result<u64> {
        let current = self.get_balance(account, token);
        let new_bal = current.checked_add(amount)
            .ok_or_else(|| anyhow::anyhow!("balance overflow for '{}' {}", account, token))?;
        self.set_balance(account, token, new_bal)?;
        Ok(new_bal)
    }

    pub fn debit(&self, account: &str, token: &str, amount: u64) -> Result<u64> {
        let bal = self.get_balance(account, token);
        anyhow::ensure!(bal >= amount, "insufficient balance: {} has {} {}", account, bal, token);
        let new_bal = bal - amount;
        self.set_balance(account, token, new_bal)?;
        Ok(new_bal)
    }

    /// Scan all token balances for an account.
    pub fn scan_balances(&self, account: &str) -> Vec<(String, u64)> {
        let Some(cf) = self.db.cf_handle(CF_BALANCES) else { return vec![] };
        let prefix = format!("{}\0", account);
        let iter = self.db.iterator_cf(&cf, IteratorMode::From(prefix.as_bytes(), Direction::Forward));
        let mut out = Vec::new();
        for item in iter {
            let Ok((k, v)) = item else { break };
            if !k.starts_with(prefix.as_bytes()) { break }
            let token = String::from_utf8_lossy(&k[prefix.len()..]).to_string();
            if let Ok(bytes) = <[u8; 8]>::try_from(v.as_ref()) {
                out.push((token, u64::from_le_bytes(bytes)));
            }
        }
        out
    }

    // ── Stakes ───────────────────────────────────────────────────────────────

    pub fn get_stake(&self, account: &str) -> u64 {
        let Some(cf) = self.db.cf_handle(CF_STAKES) else { return 0 };
        self.db.get_cf(&cf, account.as_bytes())
            .ok().flatten()
            .and_then(|b| <[u8; 8]>::try_from(b.as_ref()).ok())
            .map(u64::from_le_bytes)
            .unwrap_or(0)
    }

    pub fn set_stake(&self, account: &str, amount: u64) -> Result<()> {
        let cf = self.db.cf_handle(CF_STAKES).context("stakes CF")?;
        self.db.put_cf(&cf, account.as_bytes(), amount.to_le_bytes())?;
        Ok(())
    }

    // ── Epochs ───────────────────────────────────────────────────────────────

    pub fn set_epoch_meta(&self, epoch: u64, data: &serde_json::Value) -> Result<()> {
        let cf = self.db.cf_handle(CF_EPOCHS).context("epochs CF")?;
        self.db.put_cf(&cf, epoch.to_le_bytes(), serde_json::to_vec(data)?)?;
        Ok(())
    }

    pub fn get_epoch_meta(&self, epoch: u64) -> Result<Option<serde_json::Value>> {
        let cf = self.db.cf_handle(CF_EPOCHS).context("epochs CF")?;
        match self.db.get_cf(&cf, epoch.to_le_bytes())? {
            Some(b) => Ok(serde_json::from_slice(&b).ok()),
            None => Ok(None),
        }
    }

    // ── Metadata ─────────────────────────────────────────────────────────────

    pub fn get_meta(&self, key: &str) -> Option<Vec<u8>> {
        let cf = self.db.cf_handle(CF_META)?;
        self.db.get_cf(&cf, key.as_bytes()).ok().flatten()
    }

    pub fn set_meta(&self, key: &str, value: &[u8]) -> Result<()> {
        let cf = self.db.cf_handle(CF_META).context("meta CF")?;
        self.db.put_cf(&cf, key.as_bytes(), value)?;
        Ok(())
    }

    // ── Contract state ───────────────────────────────────────────────────────
    // Stored in CF_META with caller-supplied keys (e.g. "contract:{id}:{key}").

    pub fn state_set(&self, key: &str, value: &[u8]) -> Result<()> {
        let cf = self.db.cf_handle(CF_META).context("meta CF")?;
        self.db.put_cf(&cf, key.as_bytes(), value)?;
        Ok(())
    }

    pub fn state_get(&self, key: &str) -> Option<Vec<u8>> {
        let cf = self.db.cf_handle(CF_META)?;
        self.db.get_cf(&cf, key.as_bytes()).ok().flatten()
    }

    /// Scan all keys with the given prefix in CF_META.
    /// Returns (key, value) pairs where `key` is the full stored key string.
    pub fn state_scan_prefix(&self, prefix: &str) -> Vec<(String, Vec<u8>)> {
        let Some(cf) = self.db.cf_handle(CF_META) else { return vec![] };
        let iter = self.db.iterator_cf(
            &cf,
            IteratorMode::From(prefix.as_bytes(), Direction::Forward),
        );
        let mut out = Vec::new();
        for item in iter {
            let Ok((k, v)) = item else { break };
            if !k.starts_with(prefix.as_bytes()) {
                break;
            }
            if let Ok(key_str) = std::str::from_utf8(&k) {
                out.push((key_str.to_string(), v.to_vec()));
            }
        }
        out
    }

    pub fn batch_write<F>(&self, f: F) -> Result<()>
    where F: FnOnce(&mut WriteBatch, &DB) -> Result<()>
    {
        let mut batch = WriteBatch::default();
        f(&mut batch, &self.db)?;
        self.db.write(batch)?;
        Ok(())
    }
}

fn balance_key(account: &str, token: &str) -> Vec<u8> {
    let mut k = account.as_bytes().to_vec();
    k.push(0);
    k.extend_from_slice(token.as_bytes());
    k
}

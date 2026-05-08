//! RocksDB state store — single database, multiple column families.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;
use anyhow::{Context, Result};
use rocksdb::{DB, ColumnFamilyDescriptor, Options, WriteBatch, IteratorMode, Direction};
use sha2::{Sha256, Digest as _};

pub const CF_BLOCKS: &str = "blocks";
pub const CF_FINALITY: &str = "finality";
pub const CF_ACCOUNTS: &str = "accounts";
pub const CF_BALANCES: &str = "balances";
pub const CF_STAKES: &str = "stakes";
pub const CF_EPOCHS: &str = "epochs";
pub const CF_META: &str = "meta";
pub const CF_GIT_OBJECTS: &str = "git_objects";

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

        let cfs = [CF_BLOCKS, CF_FINALITY, CF_ACCOUNTS, CF_BALANCES, CF_STAKES, CF_EPOCHS, CF_META, CF_GIT_OBJECTS]
            .iter().map(|name| ColumnFamilyDescriptor::new(*name, Options::default()))
            .collect::<Vec<_>>();

        let db = DB::open_cf_descriptors(&opts, path, cfs)
            .with_context(|| format!("opening RocksDB at {:?}", path))?;

        Ok(Self { db: Arc::new(db) })
    }

    // ── Blocks ───────────────────────────────────────────────────────────────

    // Block and finality keys are stored big-endian (BE) so that RocksDB's
    // lexicographic iteration returns epochs in ascending order and
    // IteratorMode::End reliably returns the highest epoch.

    pub fn write_block(&self, epoch: u64, data: &[u8]) -> Result<()> {
        let cf = self.db.cf_handle(CF_BLOCKS).context("blocks CF")?;
        self.db.put_cf(&cf, epoch.to_be_bytes(), data)?;
        Ok(())
    }

    pub fn read_block(&self, epoch: u64) -> Result<Option<Vec<u8>>> {
        let cf = self.db.cf_handle(CF_BLOCKS).context("blocks CF")?;
        Ok(self.db.get_cf(&cf, epoch.to_be_bytes())?)
    }

    pub fn has_block(&self, epoch: u64) -> bool {
        let Some(cf) = self.db.cf_handle(CF_BLOCKS) else { return false };
        self.db.get_cf(&cf, epoch.to_be_bytes()).map(|v| v.is_some()).unwrap_or(false)
    }

    pub fn latest_epoch(&self) -> Option<u64> {
        let cf = self.db.cf_handle(CF_BLOCKS)?;
        let mut iter = self.db.iterator_cf(&cf, IteratorMode::End);
        // Skip any keys that aren't 8 bytes — leftover from old format.
        for item in iter.by_ref() {
            let Ok((k, _)) = item else { continue };
            if k.len() == 8 {
                return Some(u64::from_be_bytes(k[..8].try_into().unwrap()));
            }
        }
        None
    }

    pub fn write_finality(&self, epoch: u64, data: &[u8]) -> Result<()> {
        let cf = self.db.cf_handle(CF_FINALITY).context("finality CF")?;
        self.db.put_cf(&cf, epoch.to_be_bytes(), data)?;
        Ok(())
    }

    pub fn read_finality(&self, epoch: u64) -> Result<Option<Vec<u8>>> {
        let cf = self.db.cf_handle(CF_FINALITY).context("finality CF")?;
        Ok(self.db.get_cf(&cf, epoch.to_be_bytes())?)
    }

    pub fn latest_finality(&self) -> Option<u64> {
        let cf = self.db.cf_handle(CF_FINALITY)?;
        let mut iter = self.db.iterator_cf(&cf, IteratorMode::End);
        for item in iter.by_ref() {
            let Ok((k, _)) = item else { continue };
            if k.len() == 8 {
                return Some(u64::from_be_bytes(k[..8].try_into().unwrap()));
            }
        }
        None
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

    pub fn state_delete(&self, key: &str) -> Result<()> {
        let cf = self.db.cf_handle(CF_META).context("meta CF")?;
        self.db.delete_cf(&cf, key.as_bytes())?;
        Ok(())
    }

    // ── API key index ─────────────────────────────────────────────────────────

    /// Look up which account registered `token` as their API key.
    /// Returns `Some(account_name)` or `None` if the token is not registered.
    pub fn get_account_by_api_key(&self, token: &str) -> Option<String> {
        let bytes = self.state_get(&format!("api_key:{}", token))?;
        String::from_utf8(bytes).ok()
    }

    /// Register or rotate `key` as the API key for `account`.
    /// Atomically removes the old key index (if any) and installs the new one.
    pub fn set_api_key(&self, account: &str, key: &str) -> Result<()> {
        // Remove old forward index if a previous key exists.
        if let Some(old_bytes) = self.state_get(&format!("account_api_key:{}", account)) {
            if let Ok(old_key) = String::from_utf8(old_bytes) {
                let _ = self.state_delete(&format!("api_key:{}", old_key));
            }
        }
        self.state_set(&format!("api_key:{}", key), account.as_bytes())?;
        self.state_set(&format!("account_api_key:{}", account), key.as_bytes())?;
        Ok(())
    }

    // ── Chain Entropy Protocol — alive-epoch tracking ─────────────────────────

    pub fn get_alive_epoch(&self, account: &str) -> u64 {
        self.state_get(&format!("alive:{}", account))
            .and_then(|b| b.try_into().ok())
            .map(u64::from_le_bytes)
            .unwrap_or(0)
    }

    pub fn set_alive_epoch(&self, account: &str, epoch: u64) -> Result<()> {
        self.state_set(&format!("alive:{}", account), &epoch.to_le_bytes())
    }

    /// Iterate all account IDs. Used by the entropy decay sweep in finalize.
    pub fn scan_account_ids(&self) -> Vec<String> {
        let Some(cf) = self.db.cf_handle(CF_ACCOUNTS) else { return vec![] };
        let iter = self.db.iterator_cf(&cf, IteratorMode::Start);
        iter.filter_map(|item| {
            let (k, _) = item.ok()?;
            String::from_utf8(k.to_vec()).ok()
        }).collect()
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

    /// Deterministic SHA-256 hash over all balances and stakes, sorted by key.
    ///
    /// Used for replay determinism checks: two nodes that have applied the same
    /// entries in the same order must produce the same hash here.
    pub fn full_state_hash(&self) -> String {
        let mut hasher = Sha256::new();

        // Hash all balance entries sorted by (account\0token) key.
        if let Some(cf) = self.db.cf_handle(CF_BALANCES) {
            let mut entries: Vec<(Vec<u8>, Vec<u8>)> = self.db
                .iterator_cf(&cf, IteratorMode::Start)
                .filter_map(|r| r.ok().map(|(k, v)| (k.to_vec(), v.to_vec())))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, v) in entries {
                hasher.update(b"bal:");
                hasher.update(&k);
                hasher.update(b"=");
                hasher.update(&v);
                hasher.update(b"\n");
            }
        }

        // Hash all stake entries sorted by account key.
        if let Some(cf) = self.db.cf_handle(CF_STAKES) {
            let mut entries: Vec<(Vec<u8>, Vec<u8>)> = self.db
                .iterator_cf(&cf, IteratorMode::Start)
                .filter_map(|r| r.ok().map(|(k, v)| (k.to_vec(), v.to_vec())))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, v) in entries {
                hasher.update(b"stake:");
                hasher.update(&k);
                hasher.update(b"=");
                hasher.update(&v);
                hasher.update(b"\n");
            }
        }

        format!("{:x}", hasher.finalize())
    }
}

fn balance_key(account: &str, token: &str) -> Vec<u8> {
    let mut k = account.as_bytes().to_vec();
    k.push(0);
    k.extend_from_slice(token.as_bytes());
    k
}

// ── Merkle tree helpers ───────────────────────────────────────────────────────

/// Leaf hash: sha256("leaf:" || key || "=" || value_bytes)
pub fn merkle_leaf(key: &[u8], val: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"leaf:");
    h.update(key);
    h.update(b"=");
    h.update(val);
    h.finalize().into()
}

/// Internal node: sha256("node:" || left_hash || right_hash)
pub fn merkle_node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"node:");
    h.update(left.as_ref());
    h.update(right.as_ref());
    h.finalize().into()
}

/// Build a binary Merkle tree level-by-level.
/// `levels[0]` = leaf hashes (sorted), `levels.last()` = single root hash.
/// Odd-length levels duplicate the last entry so every node has a pair.
pub fn build_merkle_tree(leaves: &[[u8; 32]]) -> Vec<Vec<[u8; 32]>> {
    if leaves.is_empty() {
        return vec![];
    }
    let mut levels: Vec<Vec<[u8; 32]>> = vec![leaves.to_vec()];
    while levels.last().unwrap().len() > 1 {
        let current = levels.last().unwrap();
        let mut next = Vec::with_capacity((current.len() + 1) / 2);
        let mut i = 0;
        while i < current.len() {
            let left = current[i];
            let right = if i + 1 < current.len() { current[i + 1] } else { current[i] };
            next.push(merkle_node(&left, &right));
            i += 2;
        }
        levels.push(next);
    }
    levels
}

/// Proof path from leaf at `idx` up to the root (excluding root).
/// Each element is (sibling_hash, is_current_left_child).
pub fn merkle_proof_path(levels: &[Vec<[u8; 32]>], idx: usize) -> Vec<([u8; 32], bool)> {
    let mut path = Vec::new();
    let mut cur = idx;
    for level in &levels[..levels.len().saturating_sub(1)] {
        let sibling_idx = if cur % 2 == 0 {
            if cur + 1 < level.len() { cur + 1 } else { cur }
        } else {
            cur - 1
        };
        path.push((level[sibling_idx], cur % 2 == 0)); // is_left = cur is left child
        cur /= 2;
    }
    path
}

// ── Public proof API ──────────────────────────────────────────────────────────

/// Proof that an account holds a given balance at the current Merkle state root.
#[derive(Clone)]
pub struct BalanceMerkleProof {
    pub account: String,
    pub token: String,
    pub balance: u64,
    /// Hex-encoded leaf hash.
    pub leaf_hash: String,
    /// Proof path: (sibling_hash_hex, is_current_node_left_child).
    pub path: Vec<(String, bool)>,
    /// Hex-encoded Merkle root.
    pub root: String,
}

impl BalanceMerkleProof {
    /// Verify the proof without access to the store.
    /// Recomputes the leaf from (account, token, balance), checks it matches `leaf_hash`,
    /// then walks the sibling path to reconstruct the root.
    pub fn verify(&self) -> bool {
        // Recompute the leaf from the stated (account, token, balance).
        let key = balance_key(&self.account, &self.token);
        let val = self.balance.to_le_bytes();
        let expected_leaf = merkle_leaf(&key, &val);
        if hex::encode(expected_leaf) != self.leaf_hash {
            return false;
        }
        let mut current = expected_leaf;
        for (sib_hex, is_left) in &self.path {
            let sib_bytes = match hex::decode(sib_hex) {
                Ok(b) if b.len() == 32 => b,
                _ => return false,
            };
            let sib: [u8; 32] = sib_bytes.try_into().unwrap();
            current = if *is_left {
                merkle_node(&current, &sib)
            } else {
                merkle_node(&sib, &current)
            };
        }
        hex::encode(current) == self.root
    }
}

impl Store {
    /// Compute the Merkle root over all balance entries sorted by (account\0token) key.
    /// Returns "0000…" (64 zeros) if no balances exist.
    pub fn balance_merkle_root(&self) -> String {
        let sorted = self.sorted_balance_entries();
        if sorted.is_empty() {
            return "0".repeat(64);
        }
        let leaves: Vec<[u8; 32]> = sorted.iter()
            .map(|(k, v)| merkle_leaf(k.as_bytes(), v))
            .collect();
        let levels = build_merkle_tree(&leaves);
        hex::encode(levels.last().unwrap()[0])
    }

    /// Produce a Merkle inclusion proof for the given account's balance of `token`.
    /// Returns `None` if the account has no balance record for this token.
    pub fn balance_merkle_proof(&self, account: &str, token: &str) -> Option<BalanceMerkleProof> {
        let target_key = format!("{}\0{}", account, token);
        let sorted = self.sorted_balance_entries();
        if sorted.is_empty() {
            return None;
        }
        let idx = sorted.iter().position(|(k, _)| k == &target_key)?;
        let (_, val_bytes) = &sorted[idx];
        let balance = u64::from_le_bytes(val_bytes.as_slice().try_into().ok()?);
        let leaf = merkle_leaf(target_key.as_bytes(), val_bytes);
        let leaves: Vec<[u8; 32]> = sorted.iter()
            .map(|(k, v)| merkle_leaf(k.as_bytes(), v))
            .collect();
        let levels = build_merkle_tree(&leaves);
        let root = hex::encode(levels.last().unwrap()[0]);
        let raw_path = merkle_proof_path(&levels, idx);
        let path = raw_path.into_iter()
            .map(|(h, is_left)| (hex::encode(h), is_left))
            .collect();
        Some(BalanceMerkleProof {
            account: account.to_string(),
            token: token.to_string(),
            balance,
            leaf_hash: hex::encode(leaf),
            path,
            root,
        })
    }

    /// All balance entries sorted lexicographically by "account\0token" key.
    /// Value is 8 LE bytes (u64 dreams).
    fn sorted_balance_entries(&self) -> Vec<(String, Vec<u8>)> {
        let Some(cf) = self.db.cf_handle(CF_BALANCES) else { return vec![] };
        let mut entries: Vec<(Vec<u8>, Vec<u8>)> = self.db
            .iterator_cf(&cf, IteratorMode::Start)
            .filter_map(|r| r.ok().map(|(k, v)| (k.to_vec(), v.to_vec())))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries.into_iter()
            .filter_map(|(k, v)| {
                let key_str = String::from_utf8(k).ok()?;
                Some((key_str, v))
            })
            .collect()
    }

    // ── Git object storage ────────────────────────────────────────────────────
    // Key: "{repo_id}\x00{sha1hex}"  Value: [type_byte, raw_object_bytes...]

    pub fn git_object_put(&self, repo_id: &str, sha: &str, obj_type: u8, data: &[u8]) -> Result<()> {
        let cf = self.db.cf_handle(CF_GIT_OBJECTS).context("git_objects CF")?;
        let key = format!("{}\0{}", repo_id, sha);
        let mut value = Vec::with_capacity(1 + data.len());
        value.push(obj_type);
        value.extend_from_slice(data);
        self.db.put_cf(&cf, key.as_bytes(), &value)?;
        Ok(())
    }

    pub fn git_object_get(&self, repo_id: &str, sha: &str) -> Option<Vec<u8>> {
        let cf = self.db.cf_handle(CF_GIT_OBJECTS)?;
        let key = format!("{}\0{}", repo_id, sha);
        self.db.get_cf(&cf, key.as_bytes()).ok().flatten().map(|v| v.to_vec())
    }

    pub fn git_object_exists(&self, repo_id: &str, sha: &str) -> bool {
        let Some(cf) = self.db.cf_handle(CF_GIT_OBJECTS) else { return false };
        let key = format!("{}\0{}", repo_id, sha);
        self.db.get_cf(&cf, key.as_bytes()).ok().flatten().is_some()
    }

    /// List all stored object SHAs for a repo.
    pub fn git_objects_list(&self, repo_id: &str) -> Vec<String> {
        let Some(cf) = self.db.cf_handle(CF_GIT_OBJECTS) else { return vec![] };
        let prefix = format!("{}\0", repo_id);
        let iter = self.db.iterator_cf(&cf, IteratorMode::From(prefix.as_bytes(), Direction::Forward));
        let mut shas = Vec::new();
        for item in iter {
            let Ok((k, _)) = item else { break };
            if !k.starts_with(prefix.as_bytes()) { break }
            if let Ok(key_str) = std::str::from_utf8(&k) {
                if let Some(sha) = key_str.strip_prefix(&prefix) {
                    shas.push(sha.to_owned());
                }
            }
        }
        shas
    }
}

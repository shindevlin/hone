//! Contract storage state management.
//!
//! Each contract gets an isolated KV namespace. Keys are arbitrary byte strings.
//! Values are Borsh-serialized data written by the contract.

use std::collections::HashMap;

/// The per-contract KV store.
/// Loaded from the chain's stateStore before execution, committed after.
#[derive(Debug, Clone, Default)]
pub struct StorageKv {
    /// Current committed state (from last finalized block)
    pub committed: HashMap<Vec<u8>, Vec<u8>>,
    /// Writes pending commit (populated during execution)
    pub pending: HashMap<Vec<u8>, Vec<u8>>,
    /// Keys pending deletion
    pub pending_deletes: Vec<Vec<u8>>,
}

impl StorageKv {
    pub fn read(&self, key: &[u8]) -> Option<Vec<u8>> {
        // Pending writes shadow committed state
        self.pending.get(key)
            .or_else(|| self.committed.get(key))
            .cloned()
    }

    pub fn write(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.pending_deletes.retain(|k| k != &key);
        self.pending.insert(key, value);
    }

    pub fn delete(&mut self, key: Vec<u8>) {
        self.pending.remove(&key);
        if self.committed.contains_key(&key) {
            self.pending_deletes.push(key);
        }
    }

    pub fn has_key(&self, key: &[u8]) -> bool {
        self.read(key).is_some()
    }

    /// Return all pending writes as (key_hex, value) pairs for Node.js.
    pub fn drain_writes(&mut self) -> Vec<(String, Vec<u8>)> {
        self.pending.drain()
            .map(|(k, v)| (hex::encode(&k), v))
            .collect()
    }

    /// Return all pending deletes as hex-encoded key strings.
    pub fn drain_deletes(&mut self) -> Vec<String> {
        self.pending_deletes.drain(..)
            .map(|k| hex::encode(&k))
            .collect()
    }
}

/// Full context for a contract execution.
#[derive(Debug, Clone)]
pub struct ContractState {
    pub contract_id: String,
    pub storage: StorageKv,
    pub balance: u128,
}

impl ContractState {
    pub fn new(contract_id: String, storage: StorageKv, balance: u128) -> Self {
        Self { contract_id, storage, balance }
    }
}

//! WASM smart contract integration — deploys and calls contracts via btcpc-contract-runtime.
//!
//! State is persisted in RocksDB (CF_META) using namespaced keys:
//!   "contract:{contract_id}:{hex_storage_key}"  → raw value bytes
//!   "wasm:{contract_id}"                         → raw WASM bytecode
//!
//! The hex_storage_key corresponds to the hex-encoded raw bytes that the
//! contract runtime uses internally (drain_writes returns hex keys).

use std::sync::Arc;
use anyhow::{anyhow, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use btcpc_contract_runtime::{
    execute_call, execute_deploy, execute_view,
    CallRequest, DeployRequest,
    state::{ContractState, StorageKv},
    derive_contract_address,
};
use btcpc_types::{LedgerEntry, NATIVE_TOKEN};

use crate::chain::Chain;

pub struct ContractEngine {
    chain: Arc<Chain>,
}

impl ContractEngine {
    pub fn new(chain: Arc<Chain>) -> Self {
        Self { chain }
    }

    /// Deploy a WASM contract. Returns the contract_id.
    ///
    /// On success, all initial storage writes from the constructor are persisted
    /// to RocksDB and the raw WASM bytecode is stored under `wasm:{contract_id}`.
    pub fn deploy(
        &self,
        deployer: &str,
        wasm_b64: &str,
        init_method: Option<String>,
        init_args: Option<serde_json::Value>,
        gas: u64,
        epoch: u64,
    ) -> Result<String> {
        let wasm_bytes = BASE64
            .decode(wasm_b64)
            .map_err(|e| anyhow!("invalid base64 WASM: {}", e))?;

        let contract_id = derive_contract_address(deployer, epoch, 0);

        let request = DeployRequest {
            deployer: deployer.to_string(),
            wasm_b64: wasm_b64.to_string(),
            init_method,
            init_args,
            gas,
            epoch,
            nonce: 0,
        };

        let result = execute_deploy(request, StorageKv::default());

        if !result.success {
            return Err(anyhow!(
                "contract deploy failed: {}",
                result.error.unwrap_or_else(|| "unknown error".to_string())
            ));
        }

        // Persist constructor storage writes. Each key from drain_writes is
        // a hex-encoded representation of the raw contract storage key.
        for (hex_key, value) in &result.storage_writes {
            let store_key = format!("contract:{}:{}", contract_id, hex_key);
            self.chain.store.state_set(&store_key, value)?;
        }

        // Store raw WASM bytes separately for fast loading on future calls.
        let wasm_meta_key = format!("wasm:{}", contract_id);
        self.chain.store.set_meta(&wasm_meta_key, &wasm_bytes)?;

        Ok(contract_id)
    }

    /// Call a state-changing contract method.
    ///
    /// Loads contract storage and WASM from RocksDB, executes the call,
    /// persists storage writes, and applies any pending token transfers.
    pub fn call(
        &self,
        contract_id: &str,
        method: &str,
        args: serde_json::Value,
        signer: &str,
        gas: u64,
        deposit: u64,
        epoch: u64,
    ) -> Result<serde_json::Value> {
        let contract_state = self.load_contract_state(contract_id)?;

        let request = CallRequest {
            contract_id: contract_id.to_string(),
            method: method.to_string(),
            args,
            signer: signer.to_string(),
            predecessor: signer.to_string(),
            attached_deposit: deposit as u128,
            gas,
            epoch,
        };

        let result = execute_call(request, contract_state);

        if !result.success {
            return Err(anyhow!(
                "contract call failed: {}",
                result.error.unwrap_or_else(|| "unknown error".to_string())
            ));
        }

        // Persist storage writes back to RocksDB.
        for (hex_key, value) in &result.storage_writes {
            let store_key = format!("contract:{}:{}", contract_id, hex_key);
            self.chain.store.state_set(&store_key, value)?;
        }

        // Apply pending token transfers as Transfer ledger entries.
        for transfer in &result.pending_transfers {
            let recipient: &str = &transfer.0;
            let amount: u64 = transfer.1.min(u64::MAX as u128) as u64;
            let entry = LedgerEntry::Transfer {
                from: contract_id.to_string(),
                to: recipient.to_string(),
                amount,
                token: NATIVE_TOKEN.to_string(),
                memo: Some(format!("contract transfer from {}", contract_id)),
                epoch,
                signed_by: contract_id.to_string(),
                nonce: 0,
            };
            if let Err(e) = self.chain.apply_entry(&entry) {
                tracing::warn!(
                    "contract pending transfer failed ({} -> {} {}): {}",
                    contract_id, recipient, amount, e
                );
            }
        }

        Ok(result.result.unwrap_or(serde_json::Value::Null))
    }

    /// Call a read-only (view) contract method. State writes are discarded.
    pub fn view(
        &self,
        contract_id: &str,
        method: &str,
        args: serde_json::Value,
        gas: u64,
        epoch: u64,
    ) -> Result<serde_json::Value> {
        let contract_state = self.load_contract_state(contract_id)?;

        let request = CallRequest {
            contract_id: contract_id.to_string(),
            method: method.to_string(),
            args,
            signer: String::new(),
            predecessor: String::new(),
            attached_deposit: 0,
            gas,
            epoch,
        };

        let result = execute_view(request, contract_state);

        if !result.success {
            return Err(anyhow!(
                "contract view failed: {}",
                result.error.unwrap_or_else(|| "unknown error".to_string())
            ));
        }

        Ok(result.result.unwrap_or(serde_json::Value::Null))
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Load a ContractState from RocksDB, reconstructing the StorageKv committed map
    /// and fetching the WASM bytes (stored under the `__wasm` key as the runtime expects).
    fn load_contract_state(&self, contract_id: &str) -> Result<ContractState> {
        // Scan all persisted storage entries for this contract.
        let prefix = format!("contract:{}:", contract_id);
        let raw_entries = self.chain.store.state_scan_prefix(&prefix);

        let mut committed: std::collections::HashMap<Vec<u8>, Vec<u8>> =
            std::collections::HashMap::new();

        for (full_key, value) in raw_entries {
            // Strip the "contract:{contract_id}:" prefix to get the hex-encoded raw key.
            let hex_key = &full_key[prefix.len()..];
            let raw_key = hex::decode(hex_key)
                .map_err(|e| anyhow!("corrupted storage key '{}': {}", hex_key, e))?;
            committed.insert(raw_key, value);
        }

        // Load WASM bytes from meta and inject under the `__wasm` key that
        // execute_call expects to find in storage.committed.
        let wasm_meta_key = format!("wasm:{}", contract_id);
        let wasm_bytes = self.chain.store.get_meta(&wasm_meta_key)
            .ok_or_else(|| anyhow!("contract '{}' not found (no WASM stored)", contract_id))?;
        committed.insert(b"__wasm".to_vec(), wasm_bytes);

        let storage = StorageKv {
            committed,
            pending: std::collections::HashMap::new(),
            pending_deletes: Vec::new(),
        };

        let balance = self.chain.store.get_balance(contract_id, NATIVE_TOKEN) as u128;

        Ok(ContractState::new(contract_id.to_string(), storage, balance))
    }
}

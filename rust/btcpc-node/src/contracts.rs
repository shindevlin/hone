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
use crate::tx;

pub struct ContractEngine {
    chain: Arc<Chain>,
}

impl ContractEngine {
    pub fn new(chain: Arc<Chain>) -> Self {
        Self { chain }
    }

    /// Deploy a WASM contract. Returns the contract_id.
    ///
    /// `nonce` must be the deployer's current nonce + 1 (same scheme as Transfer).
    /// On success the deployer's nonce is bumped and all constructor storage
    /// writes are persisted to RocksDB.
    pub fn deploy(
        &self,
        deployer: &str,
        wasm_b64: &str,
        init_method: Option<String>,
        init_args: Option<serde_json::Value>,
        gas: u64,
        epoch: u64,
        nonce: u64,
    ) -> Result<String> {
        let _guard = self.chain.write_lock.lock();

        // Nonce check — same scheme as Transfer/Stake.
        let expected_nonce = match self.chain.store.get_account(deployer)? {
            Some(ref state) => state.get("nonce").and_then(|v| v.as_u64()).unwrap_or(0) + 1,
            None => 1,
        };
        if nonce != expected_nonce {
            return Err(anyhow!("invalid nonce: got {}, expected {}", nonce, expected_nonce));
        }

        let wasm_bytes = BASE64
            .decode(wasm_b64)
            .map_err(|e| anyhow!("invalid base64 WASM: {}", e))?;

        let contract_id = derive_contract_address(deployer, epoch, nonce);

        let request = DeployRequest {
            deployer: deployer.to_string(),
            wasm_b64: wasm_b64.to_string(),
            init_method,
            init_args,
            gas,
            epoch,
            nonce,
        };

        let result = execute_deploy(request, StorageKv::default());

        if !result.success {
            return Err(anyhow!(
                "contract deploy failed: {}",
                result.error.unwrap_or_else(|| "unknown error".to_string())
            ));
        }

        // Persist constructor storage writes.
        for (hex_key, value) in &result.storage_writes {
            let store_key = format!("contract:{}:{}", contract_id, hex_key);
            self.chain.store.state_set(&store_key, value)?;
        }

        // Store raw WASM bytes separately for fast loading on future calls.
        let wasm_meta_key = format!("wasm:{}", contract_id);
        self.chain.store.set_meta(&wasm_meta_key, &wasm_bytes)?;

        // Bump deployer nonce.
        tx::bump_nonce(&self.chain, deployer)?;

        Ok(contract_id)
    }

    /// Call a state-changing contract method.
    ///
    /// Loads contract storage and WASM from RocksDB, executes the call,
    /// applies all pending token transfers, and only then persists storage writes.
    ///
    /// `deposit` is debited from `signer` before execution and credited to the
    /// contract.  On any failure the deposit is refunded automatically.
    /// Storage writes are committed only after all transfers succeed.
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
        let _guard = self.chain.write_lock.lock();

        let contract_state = self.load_contract_state(contract_id)?;

        // Debit deposit from signer and credit contract before execution.
        if deposit > 0 {
            self.chain.store.debit(signer, NATIVE_TOKEN, deposit)
                .map_err(|e| anyhow!("deposit debit failed: {}", e))?;
            self.chain.store.credit(contract_id, NATIVE_TOKEN, deposit)
                .map_err(|e| {
                    let _ = self.chain.store.credit(signer, NATIVE_TOKEN, deposit);
                    anyhow!("deposit credit failed: {}", e)
                })?;
        }

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
            if deposit > 0 {
                let _ = self.chain.store.debit(contract_id, NATIVE_TOKEN, deposit);
                let _ = self.chain.store.credit(signer, NATIVE_TOKEN, deposit);
            }
            return Err(anyhow!(
                "contract call failed: {}",
                result.error.unwrap_or_else(|| "unknown error".to_string())
            ));
        }

        // Apply pending token transfers BEFORE writing storage.
        // If any transfer fails, we refund the deposit and return an error
        // without committing any storage mutations.
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
                // Refund deposit; storage not yet written so no partial state.
                if deposit > 0 {
                    let _ = self.chain.store.debit(contract_id, NATIVE_TOKEN, deposit);
                    let _ = self.chain.store.credit(signer, NATIVE_TOKEN, deposit);
                }
                return Err(anyhow!(
                    "contract pending transfer failed ({} -> {} {}): {}",
                    contract_id, recipient, amount, e
                ));
            }
        }

        // All transfers succeeded — now commit storage writes atomically.
        for (hex_key, value) in &result.storage_writes {
            let store_key = format!("contract:{}:{}", contract_id, hex_key);
            self.chain.store.state_set(&store_key, value)?;
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

    fn load_contract_state(&self, contract_id: &str) -> Result<ContractState> {
        let prefix = format!("contract:{}:", contract_id);
        let raw_entries = self.chain.store.state_scan_prefix(&prefix);

        let mut committed: std::collections::HashMap<Vec<u8>, Vec<u8>> =
            std::collections::HashMap::new();

        for (full_key, value) in raw_entries {
            let hex_key = &full_key[prefix.len()..];
            let raw_key = hex::decode(hex_key)
                .map_err(|e| anyhow!("corrupted storage key '{}': {}", hex_key, e))?;
            committed.insert(raw_key, value);
        }

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

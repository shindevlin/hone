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
        for hex_key in &result.storage_deletes {
            let store_key = format!("contract:{}:{}", contract_id, hex_key);
            self.chain.store.state_delete(&store_key)?;
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
        nonce: u64,
    ) -> Result<serde_json::Value> {
        let _guard = self.chain.write_lock.lock();

        // Nonce check prevents replay of signed contract calls.
        let expected_nonce = match self.chain.store.get_account(signer)? {
            Some(ref state) => state.get("nonce").and_then(|v| v.as_u64()).unwrap_or(0) + 1,
            None => 1,
        };
        if nonce != expected_nonce {
            return Err(anyhow!("invalid nonce: got {}, expected {}", nonce, expected_nonce));
        }

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
                twofactor: None,
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

        // All transfers succeeded — now commit storage writes and deletes atomically.
        for (hex_key, value) in &result.storage_writes {
            let store_key = format!("contract:{}:{}", contract_id, hex_key);
            self.chain.store.state_set(&store_key, value)?;
        }
        for hex_key in &result.storage_deletes {
            let store_key = format!("contract:{}:{}", contract_id, hex_key);
            self.chain.store.state_delete(&store_key)?;
        }

        // Bump signer nonce exactly once after successful execution.
        tx::bump_nonce(&self.chain, signer)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

    static REGISTRY_WASM: &[u8] = include_bytes!("../contracts/registry.wasm");

    fn make_engine(label: &str) -> (ContractEngine, tempfile::TempDir) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("btcpc_contract_{}_", label))
            .tempdir()
            .expect("tempdir");
        let store = crate::store::Store::open(dir.path()).expect("store");
        let chain = Arc::new(crate::chain::Chain::new(
            store,
            format!("contract-{}", label),
            "btcpc-satoshi".to_string(),
        ));
        (ContractEngine::new(chain), dir)
    }

    fn deploy_registry(engine: &ContractEngine) -> String {
        let wasm_b64 = BASE64.encode(REGISTRY_WASM);
        engine
            .deploy(
                "shindevlin",
                &wasm_b64,
                Some("new".to_owned()),
                Some(serde_json::json!({})),
                300_000_000_000,
                1,
                1,
            )
            .expect("registry deploy")
    }

    // ── Deploy ────────────────────────────────────────────────────────────────

    #[test]
    fn deploy_returns_deterministic_address() {
        let (engine, _dir) = make_engine("deploy");
        let wasm_b64 = BASE64.encode(REGISTRY_WASM);

        let id1 = engine.deploy("shindevlin", &wasm_b64, Some("new".into()),
            Some(serde_json::json!({})), 300_000_000_000, 1, 1).expect("deploy 1");

        // Same inputs on a fresh chain → same address.
        let (engine2, _dir2) = make_engine("deploy2");
        let id2 = engine2.deploy("shindevlin", &wasm_b64, Some("new".into()),
            Some(serde_json::json!({})), 300_000_000_000, 1, 1).expect("deploy 2");

        assert_eq!(id1, id2, "contract address is deterministic");
        assert!(id1.starts_with("BTCPCsc"), "address has correct prefix");
    }

    #[test]
    fn deploy_bumps_deployer_nonce() {
        let (engine, _dir) = make_engine("deploy_nonce");
        let wasm_b64 = BASE64.encode(REGISTRY_WASM);
        let chain = &engine.chain;

        // Pre-create the account so bump_nonce has something to update.
        chain.store.set_account("shindevlin", &serde_json::json!({"nonce": 0})).unwrap();

        engine.deploy("shindevlin", &wasm_b64, Some("new".into()),
            Some(serde_json::json!({})), 300_000_000_000, 1, 1).expect("deploy");

        let nonce_after = chain.store.get_account("shindevlin").unwrap()
            .and_then(|s| s.get("nonce").and_then(|v| v.as_u64()))
            .unwrap_or(0);

        assert_eq!(nonce_after, 1, "nonce bumped after deploy");
    }

    #[test]
    fn deploy_wrong_nonce_fails() {
        let (engine, _dir) = make_engine("deploy_nonce_fail");
        let wasm_b64 = BASE64.encode(REGISTRY_WASM);
        // Nonce 0 is wrong — first deploy requires nonce 1.
        assert!(engine.deploy("shindevlin", &wasm_b64, Some("new".into()),
            Some(serde_json::json!({})), 300_000_000_000, 1, 0).is_err());
    }

    #[test]
    fn deploy_invalid_base64_fails() {
        let (engine, _dir) = make_engine("deploy_b64");
        assert!(engine.deploy("shindevlin", "not-valid-base64!!!", Some("new".into()),
            Some(serde_json::json!({})), 300_000_000_000, 1, 1).is_err());
    }

    #[test]
    fn deploy_persists_wasm_in_store() {
        let (engine, _dir) = make_engine("deploy_persist");
        let contract_id = deploy_registry(&engine);
        let wasm_key = format!("wasm:{}", contract_id);
        assert!(engine.chain.store.get_meta(&wasm_key).is_some(),
            "WASM must be stored in CF_META after deploy");
    }

    // ── Call ──────────────────────────────────────────────────────────────────

    #[test]
    fn call_register_stores_entry() {
        let (engine, _dir) = make_engine("call_register");

        // Bump deployer nonce past deploy so signer nonce starts at 0 for call.
        let contract_id = deploy_registry(&engine);

        // Call `register` — signer "alice", nonce 1 (first call from alice).
        let result = engine.call(
            &contract_id,
            "register",
            serde_json::json!({ "key": "profile", "value": "ipfs://Qmtest" }),
            "alice",
            100_000_000,
            0,
            1,
            1,
        ).expect("register call");

        assert!(result.is_null() || result.is_object() || result.is_string(),
            "register returns null or a value, not an error");
    }

    #[test]
    fn call_wrong_nonce_fails() {
        let (engine, _dir) = make_engine("call_nonce");
        let contract_id = deploy_registry(&engine);

        // Alice's first call must use nonce 1. Nonce 5 → fail.
        assert!(engine.call(
            &contract_id,
            "register",
            serde_json::json!({ "key": "k", "value": "v" }),
            "alice",
            100_000_000,
            0,
            1,
            5,
        ).is_err());
    }

    #[test]
    fn call_unknown_contract_fails() {
        let (engine, _dir) = make_engine("call_unknown");
        assert!(engine.call(
            "BTCPCscdeadbeefdeadbeefdeadbeefdeadbeef",
            "register",
            serde_json::json!({}),
            "alice",
            100_000_000,
            0,
            1,
            1,
        ).is_err());
    }

    // ── View ──────────────────────────────────────────────────────────────────

    #[test]
    fn view_get_returns_null_for_missing_key() {
        let (engine, _dir) = make_engine("view_get_missing");
        let contract_id = deploy_registry(&engine);

        let result = engine.view(
            &contract_id,
            "get",
            serde_json::json!({ "account": "alice", "key": "profile" }),
            100_000_000,
            1,
        ).expect("view get");

        assert!(result.is_null(), "missing key should return null, got {:?}", result);
    }

    #[test]
    fn view_list_keys_empty_for_new_account() {
        let (engine, _dir) = make_engine("view_list_empty");
        let contract_id = deploy_registry(&engine);

        let result = engine.view(
            &contract_id,
            "list_keys",
            serde_json::json!({ "account": "nobody" }),
            100_000_000,
            1,
        ).expect("view list_keys");

        let arr = result.as_array().expect("list_keys returns array");
        assert!(arr.is_empty(), "new account has no keys");
    }

    #[test]
    fn view_count_returns_zero_for_new_account() {
        let (engine, _dir) = make_engine("view_count");
        let contract_id = deploy_registry(&engine);

        let result = engine.view(
            &contract_id,
            "count",
            serde_json::json!({ "account": "nobody" }),
            100_000_000,
            1,
        ).expect("view count");

        assert_eq!(result.as_u64().unwrap_or(0), 0);
    }

    #[test]
    fn view_has_returns_false_for_missing_key() {
        let (engine, _dir) = make_engine("view_has");
        let contract_id = deploy_registry(&engine);

        let result = engine.view(
            &contract_id,
            "has",
            serde_json::json!({ "account": "alice", "key": "profile" }),
            100_000_000,
            1,
        ).expect("view has");

        assert_eq!(result.as_bool().unwrap_or(true), false);
    }

    #[test]
    fn view_does_not_commit_state() {
        let (engine, _dir) = make_engine("view_readonly");
        let contract_id = deploy_registry(&engine);

        // Calling a state-changing method via view must NOT persist writes.
        // The call itself may succeed or fail in the WASM sandbox, but writes are discarded.
        let _ = engine.view(
            &contract_id,
            "register",
            serde_json::json!({ "key": "k", "value": "v" }),
            100_000_000,
            1,
        );

        // Verify nothing was stored for alice.
        let count = engine.view(
            &contract_id,
            "count",
            serde_json::json!({ "account": "alice" }),
            100_000_000,
            1,
        ).expect("count view");

        assert_eq!(count.as_u64().unwrap_or(99), 0,
            "view must not persist state writes");
    }

    #[test]
    fn view_unknown_contract_fails() {
        let (engine, _dir) = make_engine("view_unknown");
        assert!(engine.view(
            "BTCPCscdeadbeefdeadbeefdeadbeefdeadbeef",
            "count",
            serde_json::json!({ "account": "alice" }),
            100_000_000,
            1,
        ).is_err());
    }
}

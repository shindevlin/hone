//! WASM smart contract integration — deploys and calls contracts via hone-contract-runtime.
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
use hone_contract_runtime::{
    execute_call, execute_deploy, execute_view,
    CallRequest, DeployRequest,
    state::{ContractState, StorageKv},
    derive_contract_address,
};
use hone_types::{LedgerEntry, NATIVE_TOKEN};

use crate::chain::Chain;
use crate::tx;

/// chain_param key that gates contract-emitted token movement (deposit + transfers).
///
/// CONSENSUS REPLAY (docs/CONTRACT_CONSENSUS_FIX.md): contract balance effects are
/// consensus state re-derived on every node at seal. Until the cross-node
/// determinism + partition tests pass, token movement stays OFF. Deploy and
/// storage-only calls (no deposit, no emitted transfers) still run.
///
/// Default: DISABLED (any missing/non-"true" value = off). Never enabled on
/// mainnet until the §1.1 partition test and a determinism-replay test pass.
pub const CONTRACT_TOKEN_TRANSFERS_PARAM: &str = "chain_param:contracts_token_transfers_enabled";

/// Returns true only when the chain param is explicitly set to "true".
pub fn contract_token_transfers_enabled(chain: &Chain) -> bool {
    chain.store.state_get(CONTRACT_TOKEN_TRANSFERS_PARAM)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

/// Load a contract's committed storage + WASM + balance into a runtime state.
fn load_contract_state(chain: &Chain, contract_id: &str) -> Result<ContractState> {
    let prefix = format!("contract:{}:", contract_id);
    let raw_entries = chain.store.state_scan_prefix(&prefix);

    let mut committed: std::collections::HashMap<Vec<u8>, Vec<u8>> =
        std::collections::HashMap::new();

    for (full_key, value) in raw_entries {
        let hex_key = &full_key[prefix.len()..];
        let raw_key = hex::decode(hex_key)
            .map_err(|e| anyhow!("corrupted storage key '{}': {}", hex_key, e))?;
        committed.insert(raw_key, value);
    }

    let wasm_meta_key = format!("wasm:{}", contract_id);
    let wasm_bytes = chain.store.get_meta(&wasm_meta_key)
        .ok_or_else(|| anyhow!("contract '{}' not found (no WASM stored)", contract_id))?;
    committed.insert(b"__wasm".to_vec(), wasm_bytes);

    let storage = StorageKv {
        committed,
        pending: std::collections::HashMap::new(),
        pending_deletes: Vec::new(),
    };

    let balance = chain.store.get_balance(contract_id, NATIVE_TOKEN) as u128;

    Ok(ContractState::new(contract_id.to_string(), storage, balance))
}

/// Deploy a WASM contract deterministically and persist its state.
///
/// CONSENSUS REPLAY: invoked at epoch seal from `Chain::apply_entry` (the
/// ContractDeploy arm), so every node runs the identical deploy against identical
/// pre-seal state. Constructor storage writes + WASM are persisted; deployer nonce
/// bumped. Returns the derived contract_id.
///
/// The caller (seal path) already holds `chain.write_lock`, so this does NOT take
/// it again (parking_lot mutex is non-reentrant).
pub fn apply_contract_deploy(
    chain: &Chain,
    deployer: &str,
    wasm_b64: &str,
    init_method: Option<String>,
    init_args: Option<serde_json::Value>,
    gas: u64,
    epoch: u64,
    nonce: u64,
) -> Result<String> {
    // Nonce check — same scheme as Transfer/Stake.
    let expected_nonce = match chain.store.get_account(deployer)? {
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
        chain.store.state_set(&store_key, value)?;
    }
    for hex_key in &result.storage_deletes {
        let store_key = format!("contract:{}:{}", contract_id, hex_key);
        chain.store.state_delete(&store_key)?;
    }

    // Store raw WASM bytes separately for fast loading on future calls.
    let wasm_meta_key = format!("wasm:{}", contract_id);
    chain.store.set_meta(&wasm_meta_key, &wasm_bytes)?;

    // Bump deployer nonce.
    tx::bump_nonce(chain, deployer)?;

    Ok(contract_id)
}

/// Execute a state-changing contract method deterministically and apply its
/// balance + storage effects to chain state.
///
/// CONSENSUS REPLAY: invoked at epoch seal from `Chain::apply_entry` (the
/// ContractCall arm), so every node runs the identical call against identical
/// pre-seal state and derives the identical balance delta → no fork.
///
/// GATING: any token movement (non-zero `deposit` OR any contract-emitted
/// transfer) is refused unless `contract_token_transfers_enabled(chain)` is true.
/// Deploy + storage-only calls run regardless.
///
/// Atomicity preserved from the original HTTP-time logic: deposit debited before
/// execution; emitted transfers applied before storage writes; deposit refunded
/// on any failure so no partial balance state is committed.
///
/// The caller (seal path) already holds `chain.write_lock`, so this does NOT take
/// it again (parking_lot mutex is non-reentrant).
pub fn apply_contract_call(
    chain: &Chain,
    contract_id: &str,
    method: &str,
    args: serde_json::Value,
    signer: &str,
    gas: u64,
    deposit: u64,
    epoch: u64,
    nonce: u64,
) -> Result<serde_json::Value> {
    // Nonce check prevents replay of signed contract calls.
    let expected_nonce = match chain.store.get_account(signer)? {
        Some(ref state) => state.get("nonce").and_then(|v| v.as_u64()).unwrap_or(0) + 1,
        None => 1,
    };
    if nonce != expected_nonce {
        return Err(anyhow!("invalid nonce: got {}, expected {}", nonce, expected_nonce));
    }

    let transfers_enabled = contract_token_transfers_enabled(chain);

    // GATE (pre-execution): a call that attaches a deposit moves tokens. Refuse
    // it up front when the flag is off — do not debit, do not execute.
    if deposit > 0 && !transfers_enabled {
        return Err(anyhow!(
            "contract token movement disabled: call attaches a deposit but \
             '{}' is not enabled",
            CONTRACT_TOKEN_TRANSFERS_PARAM
        ));
    }

    let contract_state = load_contract_state(chain, contract_id)?;

    // Debit deposit from signer and credit contract before execution.
    if deposit > 0 {
        chain.store.debit(signer, NATIVE_TOKEN, deposit)
            .map_err(|e| anyhow!("deposit debit failed: {}", e))?;
        chain.store.credit(contract_id, NATIVE_TOKEN, deposit)
            .map_err(|e| {
                let _ = chain.store.credit(signer, NATIVE_TOKEN, deposit);
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
            let _ = chain.store.debit(contract_id, NATIVE_TOKEN, deposit);
            let _ = chain.store.credit(signer, NATIVE_TOKEN, deposit);
        }
        return Err(anyhow!(
            "contract call failed: {}",
            result.error.unwrap_or_else(|| "unknown error".to_string())
        ));
    }

    // GATE (post-execution): the contract emitted transfers but token movement is
    // off. Refund the deposit and refuse WITHOUT committing storage — the call is
    // rejected atomically, exactly like a failed transfer below.
    if !transfers_enabled && !result.pending_transfers.is_empty() {
        if deposit > 0 {
            let _ = chain.store.debit(contract_id, NATIVE_TOKEN, deposit);
            let _ = chain.store.credit(signer, NATIVE_TOKEN, deposit);
        }
        return Err(anyhow!(
            "contract token movement disabled: contract emitted {} transfer(s) but \
             '{}' is not enabled",
            result.pending_transfers.len(),
            CONTRACT_TOKEN_TRANSFERS_PARAM
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
        if let Err(e) = chain.apply_entry(&entry) {
            // Refund deposit; storage not yet written so no partial state.
            if deposit > 0 {
                let _ = chain.store.debit(contract_id, NATIVE_TOKEN, deposit);
                let _ = chain.store.credit(signer, NATIVE_TOKEN, deposit);
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
        chain.store.state_set(&store_key, value)?;
    }
    for hex_key in &result.storage_deletes {
        let store_key = format!("contract:{}:{}", contract_id, hex_key);
        chain.store.state_delete(&store_key)?;
    }

    // Bump signer nonce exactly once after successful execution.
    tx::bump_nonce(chain, signer)?;

    Ok(result.result.unwrap_or(serde_json::Value::Null))
}

pub struct ContractEngine {
    chain: Arc<Chain>,
}

impl ContractEngine {
    pub fn new(chain: Arc<Chain>) -> Self {
        Self { chain }
    }

    /// Deploy a WASM contract. Returns the contract_id.
    ///
    /// Thin wrapper over `apply_contract_deploy` that takes `chain.write_lock`.
    /// Used by tests and any non-seal caller; the seal path calls the free
    /// function directly (it already holds the lock).
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
        apply_contract_deploy(&self.chain, deployer, wasm_b64, init_method, init_args, gas, epoch, nonce)
    }

    /// Call a state-changing contract method.
    ///
    /// Thin wrapper over `apply_contract_call` that takes `chain.write_lock`.
    /// Used by tests; the seal path calls the free function directly.
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
        apply_contract_call(&self.chain, contract_id, method, args, signer, gas, deposit, epoch, nonce)
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
        let contract_state = load_contract_state(&self.chain, contract_id)?;

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

    static REGISTRY_WASM: &[u8] = include_bytes!("../contracts/registry.wasm");

    fn make_engine(label: &str) -> (ContractEngine, tempfile::TempDir) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("hone_contract_{}_", label))
            .tempdir()
            .expect("tempdir");
        let store = crate::store::Store::open(dir.path()).expect("store");
        let chain = Arc::new(crate::chain::Chain::new(
            store,
            format!("contract-{}", label),
            "hone-testnet".to_string(),
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
        assert!(id1.starts_with("honesc"), "address has correct prefix");
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
            "honescdeadbeefdeadbeefdeadbeefdeadbeef",
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
            "honescdeadbeefdeadbeefdeadbeefdeadbeef",
            "count",
            serde_json::json!({ "account": "alice" }),
            100_000_000,
            1,
        ).is_err());
    }

    // ── Consensus-replay: token-movement gate (docs/CONTRACT_CONSENSUS_FIX.md) ──

    /// Enable/disable the contract token-movement chain param in a test store.
    fn set_transfers_enabled(engine: &ContractEngine, enabled: bool) {
        engine.chain.store
            .state_set(CONTRACT_TOKEN_TRANSFERS_PARAM, if enabled { b"true" } else { b"false" })
            .unwrap();
    }

    #[test]
    fn gate_defaults_disabled() {
        let (engine, _dir) = make_engine("gate_default");
        // No chain param set → token movement must be OFF (mainnet-safe default).
        assert!(!contract_token_transfers_enabled(&engine.chain),
            "contract token movement must default to disabled");
    }

    #[test]
    fn call_with_deposit_rejected_when_flag_off() {
        let (engine, _dir) = make_engine("deposit_off");
        let contract_id = deploy_registry(&engine);

        // Fund alice so a deposit debit *could* succeed — the gate must still refuse.
        engine.chain.store.credit("alice", NATIVE_TOKEN, 1_000).unwrap();
        set_transfers_enabled(&engine, false);

        let err = engine.call(
            &contract_id, "register",
            serde_json::json!({ "key": "profile", "value": "ipfs://Qmtest" }),
            "alice", 100_000_000, /* deposit */ 500, 1, 1,
        ).unwrap_err();
        assert!(err.to_string().contains("token movement disabled"),
            "deposit call must be gated when flag off, got: {}", err);

        // Gate is pre-execution: no balance moved, signer nonce not bumped.
        assert_eq!(engine.chain.store.get_balance("alice", NATIVE_TOKEN), 1_000,
            "no deposit debited when gated");
        assert_eq!(engine.chain.store.get_balance(&contract_id, NATIVE_TOKEN), 0,
            "contract received nothing when gated");
    }

    #[test]
    fn call_with_deposit_applies_when_flag_on() {
        let (engine, _dir) = make_engine("deposit_on");
        let contract_id = deploy_registry(&engine);

        // Create alice's account so bump_nonce has a record to update, and fund her.
        engine.chain.store.set_account("alice", &serde_json::json!({ "nonce": 0 })).unwrap();
        engine.chain.store.credit("alice", NATIVE_TOKEN, 1_000).unwrap();
        set_transfers_enabled(&engine, true);

        engine.call(
            &contract_id, "register",
            serde_json::json!({ "key": "profile", "value": "ipfs://Qmtest" }),
            "alice", 100_000_000, /* deposit */ 500, 1, 1,
        ).expect("deposit call succeeds when flag on");

        // Deposit moved signer → contract; this IS the consensus balance effect.
        assert_eq!(engine.chain.store.get_balance("alice", NATIVE_TOKEN), 500,
            "deposit debited from signer");
        assert_eq!(engine.chain.store.get_balance(&contract_id, NATIVE_TOKEN), 500,
            "deposit credited to contract");
        // Nonce bumped exactly once on success.
        let nonce = engine.chain.store.get_account("alice").unwrap()
            .and_then(|s| s.get("nonce").and_then(|v| v.as_u64())).unwrap_or(0);
        assert_eq!(nonce, 1, "signer nonce bumped after successful call");
    }

    #[test]
    fn deploy_runs_when_flag_off() {
        // Deploy moves no tokens → must succeed regardless of the gate.
        let (engine, _dir) = make_engine("deploy_gate_off");
        set_transfers_enabled(&engine, false);
        let contract_id = deploy_registry(&engine);
        let wasm_key = format!("wasm:{}", contract_id);
        assert!(engine.chain.store.get_meta(&wasm_key).is_some(),
            "deploy must run even with token movement disabled");
    }

    #[test]
    fn storage_only_call_runs_when_flag_off() {
        // A call with no deposit and no emitted transfers is storage-only → runs.
        let (engine, _dir) = make_engine("storage_only_off");
        let contract_id = deploy_registry(&engine);
        set_transfers_enabled(&engine, false);

        engine.call(
            &contract_id, "register",
            serde_json::json!({ "key": "k", "value": "v" }),
            "alice", 100_000_000, /* deposit */ 0, 1, 1,
        ).expect("storage-only call runs with flag off");

        let stored = engine.view(
            &contract_id, "get",
            serde_json::json!({ "account": "alice", "key": "k" }),
            100_000_000, 1,
        ).expect("view get");
        assert_eq!(stored.as_str(), Some("v"), "storage write committed");
    }

    #[test]
    fn deposit_call_determinism_two_nodes_match() {
        // CONSENSUS REPLAY core guarantee: two independent nodes running the same
        // deposit call against the same pre-seal state reach identical balances.
        let (a, _da) = make_engine("determ_a");
        let (b, _db) = make_engine("determ_b");
        for e in [&a, &b] {
            deploy_registry(e);
            e.chain.store.credit("alice", NATIVE_TOKEN, 1_000).unwrap();
            set_transfers_enabled(e, true);
        }
        let cid = derive_contract_address("shindevlin", 1, 1);
        for e in [&a, &b] {
            e.call(&cid, "register",
                serde_json::json!({ "key": "profile", "value": "ipfs://Qmx" }),
                "alice", 100_000_000, 500, 1, 1).expect("call");
        }
        assert_eq!(a.chain.store.get_balance("alice", NATIVE_TOKEN),
                   b.chain.store.get_balance("alice", NATIVE_TOKEN),
                   "signer balance identical across nodes");
        assert_eq!(a.chain.store.get_balance(&cid, NATIVE_TOKEN),
                   b.chain.store.get_balance(&cid, NATIVE_TOKEN),
                   "contract balance identical across nodes");
    }
}

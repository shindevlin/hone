//! WASM execution engine.

use std::sync::{Arc, Mutex};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use wasmtime::{Config, Engine, Linker, Module, Store};

use crate::{
    gas,
    host::{ExecutionContext, Ctx, register_host_functions},
    state::{ContractState, StorageKv},
    CallRequest, CallResult, DeployRequest,
};

/// Execute a state-changing contract call.
pub fn execute_call(request: CallRequest, contract_state: ContractState) -> CallResult {
    let args_bytes = serde_json::to_vec(&request.args).unwrap_or_default();
    let method = request.method.clone();

    let wasm_bytes = contract_state.storage.committed
        .get(b"__wasm".as_slice())
        .cloned()
        .unwrap_or_default();

    let ctx = Arc::new(Mutex::new(ExecutionContext {
        signer: request.signer,
        predecessor: request.predecessor,
        contract_id: contract_state.contract_id.clone(),
        epoch: request.epoch,
        attached_deposit: request.attached_deposit,
        gas_limit: request.gas,
        gas_used: 0,
        storage: contract_state.storage,
        balances: {
            let mut m = std::collections::HashMap::new();
            m.insert(contract_state.contract_id.clone(), contract_state.balance);
            m
        },
        reserved_balance: 0,
        pending_transfers: Vec::new(),
        events: Vec::new(),
        logs: Vec::new(),
        return_value: None,
        panic_message: None,
        registers: std::collections::HashMap::new(),
        input: args_bytes,
    }));

    run_wasm(wasm_bytes, method, ctx.clone(), request.gas)
}

/// Execute a read-only view call (no state changes committed).
pub fn execute_view(request: CallRequest, contract_state: ContractState) -> CallResult {
    let mut result = execute_call(request, contract_state);
    // View calls: discard any state writes (read-only contract sees real state)
    result.storage_writes.clear();
    result
}

/// Deploy a contract: validate WASM, run constructor, return initial state.
pub fn execute_deploy(request: DeployRequest, initial_storage: StorageKv) -> CallResult {
    let wasm_bytes = match BASE64.decode(&request.wasm_b64) {
        Ok(b) => b,
        Err(e) => return error_result(format!("Invalid base64 WASM: {}", e)),
    };

    let init_method = request.init_method.unwrap_or_else(|| "new".to_string());
    let args = request.init_args.unwrap_or(serde_json::Value::Null);
    let args_bytes = serde_json::to_vec(&args).unwrap_or_default();

    let contract_id = crate::derive_contract_address(&request.deployer, request.epoch, request.nonce);

    let mut storage = initial_storage;
    // Store WASM bytecode in contract storage (under __wasm key)
    storage.write(b"__wasm".to_vec(), wasm_bytes.clone());

    let ctx = Arc::new(Mutex::new(ExecutionContext {
        signer: request.deployer.clone(),
        predecessor: request.deployer.clone(),
        contract_id: contract_id.clone(),
        epoch: request.epoch,
        attached_deposit: 0,
        gas_limit: request.gas,
        gas_used: 0,
        storage,
        balances: std::collections::HashMap::new(),
        reserved_balance: 0,
        pending_transfers: Vec::new(),
        events: Vec::new(),
        logs: Vec::new(),
        return_value: None,
        panic_message: None,
        registers: std::collections::HashMap::new(),
        input: args_bytes,
    }));

    run_wasm(wasm_bytes, init_method, ctx.clone(), request.gas)
}

// ── Internal ─────────────────────────────────────────────────────────────────

fn run_wasm(wasm_bytes: Vec<u8>, method: String, ctx: Ctx, gas: u64) -> CallResult {
    // consume_fuel must be enabled on the Engine before fuel can be set on the Store.
    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = match Engine::new(&config) {
        Ok(e) => e,
        Err(e) => return error_result(format!("Engine init failed: {}", e)),
    };
    let module = match Module::from_binary(&engine, &wasm_bytes) {
        Ok(m) => m,
        Err(e) => return error_result(format!("WASM compile error: {}", e)),
    };

    let mut linker: Linker<Ctx> = Linker::new(&engine);
    if let Err(e) = register_host_functions(&mut linker) {
        return error_result(format!("Host function registration failed: {}", e));
    }

    let fuel_limit = gas.min(gas::MAX_CALL_FUEL);
    let mut store = Store::new(&engine, ctx.clone());
    // Hard-fail if fuel cannot be applied — means consume_fuel was not enabled.
    if let Err(e) = store.set_fuel(fuel_limit) {
        return error_result(format!("Fuel setup failed (gas enforcement unavailable): {}", e));
    }

    // Set method name in register 0 before dispatch
    ctx.lock().unwrap().set_register(0, method.as_bytes().to_vec());

    let instance = match linker.instantiate(&mut store, &module) {
        Ok(i) => i,
        Err(e) => return error_result(format!("Instantiation failed: {}", e)),
    };

    let dispatch = match instance.get_typed_func::<(), ()>(&mut store, "__hone_dispatch") {
        Ok(f) => f,
        Err(_) => return error_result(format!("Contract missing __hone_dispatch export")),
    };

    let exec_result = dispatch.call(&mut store, ());
    let remaining = store.get_fuel().unwrap_or(0);
    let gas_used = fuel_limit.saturating_sub(remaining);

    let mut locked = ctx.lock().unwrap();

    match exec_result {
        Ok(_) => {
            let return_value = locked.return_value.take()
                .and_then(|b| serde_json::from_slice(&b).ok());
            let storage_writes = locked.storage.drain_writes();
            let storage_deletes = locked.storage.drain_deletes();
            let pending_transfers = std::mem::take(&mut locked.pending_transfers);
            let events = locked.events.clone();
            let logs = locked.logs.clone();

            CallResult {
                success: true,
                result: return_value,
                gas_used,
                storage_writes,
                storage_deletes,
                pending_transfers,
                events,
                logs,
                error: None,
            }
        }
        Err(e) => {
            let msg = locked.panic_message.take().unwrap_or_else(|| e.to_string());
            CallResult {
                success: false,
                result: None,
                gas_used,
                storage_writes: Vec::new(),
                storage_deletes: Vec::new(),
                pending_transfers: Vec::new(),
                events: Vec::new(),
                logs: locked.logs.clone(),
                error: Some(msg),
            }
        }
    }
}

fn error_result(msg: String) -> CallResult {
    CallResult {
        success: false,
        result: None,
        gas_used: 0,
        storage_writes: Vec::new(),
        storage_deletes: Vec::new(),
        pending_transfers: Vec::new(),
        events: Vec::new(),
        logs: Vec::new(),
        error: Some(msg),
    }
}

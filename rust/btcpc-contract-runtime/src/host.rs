//! Host function implementations — the bridge between the WASM contract and the chain.
//!
//! All state mutations are buffered in [`ExecutionContext`] and only committed
//! to the real chain state if the call succeeds (no trap, no panic).
//!
//! Note on u128 balances: WebAssembly has no 128-bit integer type, so balances
//! cross the WASM boundary as u64 (satoshi-level values). Internal Rust logic
//! still uses u128 for intermediate arithmetic.

use std::sync::{Arc, Mutex};
use anyhow::Result;
use wasmtime::Linker;

use crate::state::StorageKv;

/// Mutable context shared between host functions during a single call.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub signer: String,
    pub predecessor: String,
    pub contract_id: String,
    pub epoch: u64,
    pub attached_deposit: u128,
    pub gas_limit: u64,
    pub gas_used: u64,
    pub storage: StorageKv,
    pub balances: std::collections::HashMap<String, u128>,
    /// Sum of all amounts already enqueued via `transfer` this execution.
    /// Prevents the contract from promising more than it holds.
    pub reserved_balance: u128,
    pub pending_transfers: Vec<(String, u128)>,
    pub events: Vec<serde_json::Value>,
    pub logs: Vec<String>,
    pub return_value: Option<Vec<u8>>,
    pub panic_message: Option<String>,
    pub registers: std::collections::HashMap<u64, Vec<u8>>,
    pub input: Vec<u8>,
}

impl ExecutionContext {
    pub fn set_register(&mut self, id: u64, data: Vec<u8>) {
        self.registers.insert(id, data);
    }

    pub fn get_register_len(&self, id: u64) -> u64 {
        if id == u64::MAX {
            return self.input.len() as u64;
        }
        self.registers.get(&id).map(|v| v.len() as u64).unwrap_or(u64::MAX)
    }
}

pub type Ctx = Arc<Mutex<ExecutionContext>>;

/// Register all host functions with a wasmtime Linker.
pub fn register_host_functions(linker: &mut Linker<Ctx>) -> Result<()> {
    // Rust `extern "C"` blocks in wasm32 targets export under the "env" module by default.
    let module = "env";

    // ── Identity ─────────────────────────────────────────────────────────────

    linker.func_wrap(module, "env_signer", |caller: wasmtime::Caller<'_, Ctx>, register_id: u64| {
        let bytes = caller.data().lock().unwrap().signer.as_bytes().to_vec();
        caller.data().lock().unwrap().set_register(register_id, bytes);
    })?;

    linker.func_wrap(module, "env_predecessor", |caller: wasmtime::Caller<'_, Ctx>, register_id: u64| {
        let bytes = caller.data().lock().unwrap().predecessor.as_bytes().to_vec();
        caller.data().lock().unwrap().set_register(register_id, bytes);
    })?;

    linker.func_wrap(module, "env_contract_id", |caller: wasmtime::Caller<'_, Ctx>, register_id: u64| {
        let bytes = caller.data().lock().unwrap().contract_id.as_bytes().to_vec();
        caller.data().lock().unwrap().set_register(register_id, bytes);
    })?;

    // ── Chain state ───────────────────────────────────────────────────────────

    linker.func_wrap(module, "env_epoch", |caller: wasmtime::Caller<'_, Ctx>| -> u64 {
        caller.data().lock().unwrap().epoch
    })?;

    // Returns attached deposit in satoshis (u64, capped at u64::MAX).
    linker.func_wrap(module, "env_attached_deposit", |caller: wasmtime::Caller<'_, Ctx>| -> u64 {
        let deposit = caller.data().lock().unwrap().attached_deposit;
        deposit.min(u64::MAX as u128) as u64
    })?;

    // ── Storage ───────────────────────────────────────────────────────────────

    linker.func_wrap(module, "storage_write", |mut caller: wasmtime::Caller<'_, Ctx>,
        key_ptr: u64, key_len: u64, val_ptr: u64, val_len: u64| -> u64
    {
        let mem = caller.get_export("memory").and_then(|e| e.into_memory());
        if let Some(mem) = mem {
            let data = mem.data(&caller);
            let key_end = key_ptr.checked_add(key_len).filter(|&e| e as usize <= data.len());
            let val_end = val_ptr.checked_add(val_len).filter(|&e| e as usize <= data.len());
            if let (Some(ke), Some(ve)) = (key_end, val_end) {
                let key = data[key_ptr as usize..ke as usize].to_vec();
                let val = data[val_ptr as usize..ve as usize].to_vec();
                caller.data().lock().unwrap().storage.write(key, val);
                1
            } else { 0 }
        } else { 0 }
    })?;

    linker.func_wrap(module, "storage_read", |mut caller: wasmtime::Caller<'_, Ctx>,
        key_ptr: u64, key_len: u64, register_id: u64| -> u64
    {
        let mem = caller.get_export("memory").and_then(|e| e.into_memory());
        if let Some(mem) = mem {
            let data = mem.data(&caller);
            let key_end = key_ptr.checked_add(key_len).filter(|&e| e as usize <= data.len());
            if let Some(ke) = key_end {
                let key = data[key_ptr as usize..ke as usize].to_vec();
                let ctx = caller.data().clone();
                let value = ctx.lock().unwrap().storage.read(&key);
                if let Some(v) = value {
                    ctx.lock().unwrap().set_register(register_id, v);
                    1
                } else { 0 }
            } else { 0 }
        } else { 0 }
    })?;

    linker.func_wrap(module, "storage_remove", |mut caller: wasmtime::Caller<'_, Ctx>,
        key_ptr: u64, key_len: u64, _register_id: u64| -> u64
    {
        let mem = caller.get_export("memory").and_then(|e| e.into_memory());
        if let Some(mem) = mem {
            let data = mem.data(&caller);
            let key_end = key_ptr.checked_add(key_len).filter(|&e| e as usize <= data.len());
            if let Some(ke) = key_end {
                let key = data[key_ptr as usize..ke as usize].to_vec();
                caller.data().lock().unwrap().storage.delete(key);
                1
            } else { 0 }
        } else { 0 }
    })?;

    linker.func_wrap(module, "storage_has_key", |mut caller: wasmtime::Caller<'_, Ctx>,
        key_ptr: u64, key_len: u64| -> u64
    {
        let mem = caller.get_export("memory").and_then(|e| e.into_memory());
        if let Some(mem) = mem {
            let data = mem.data(&caller);
            let key_end = key_ptr.checked_add(key_len).filter(|&e| e as usize <= data.len());
            if let Some(ke) = key_end {
                let key = data[key_ptr as usize..ke as usize].to_vec();
                if caller.data().lock().unwrap().storage.has_key(&key) { 1 } else { 0 }
            } else { 0 }
        } else { 0 }
    })?;

    // ── Registers ─────────────────────────────────────────────────────────────

    linker.func_wrap(module, "register_len", |caller: wasmtime::Caller<'_, Ctx>, register_id: u64| -> u64 {
        caller.data().lock().unwrap().get_register_len(register_id)
    })?;

    linker.func_wrap(module, "read_register", |mut caller: wasmtime::Caller<'_, Ctx>,
        register_id: u64, ptr: u64|
    {
        let ctx = caller.data().clone();
        let data = if register_id == u64::MAX {
            ctx.lock().unwrap().input.clone()
        } else {
            ctx.lock().unwrap().registers.get(&register_id).cloned().unwrap_or_default()
        };
        let mem = caller.get_export("memory").and_then(|e| e.into_memory());
        if let Some(mem) = mem {
            let mem_data = mem.data_mut(&mut caller);
            let start = ptr as usize;
            let end = start.checked_add(data.len());
            if let Some(end) = end {
                if end <= mem_data.len() {
                    mem_data[start..end].copy_from_slice(&data);
                }
            }
        }
    })?;

    // ── Output ────────────────────────────────────────────────────────────────

    linker.func_wrap(module, "value_return", |mut caller: wasmtime::Caller<'_, Ctx>,
        value_ptr: u64, value_len: u64|
    {
        let mem = caller.get_export("memory").and_then(|e| e.into_memory());
        if let Some(mem) = mem {
            let data = mem.data(&caller);
            let end = value_ptr.checked_add(value_len).filter(|&e| e as usize <= data.len());
            if let Some(e) = end {
                let value = data[value_ptr as usize..e as usize].to_vec();
                caller.data().lock().unwrap().return_value = Some(value);
            }
        }
    })?;

    linker.func_wrap(module, "emit_event", |mut caller: wasmtime::Caller<'_, Ctx>,
        event_ptr: u64, event_len: u64|
    {
        let mem = caller.get_export("memory").and_then(|e| e.into_memory());
        if let Some(mem) = mem {
            let data = mem.data(&caller);
            let end = event_ptr.checked_add(event_len).filter(|&e| e as usize <= data.len());
            if let Some(e) = end {
                if let Ok(event) = serde_json::from_slice::<serde_json::Value>(&data[event_ptr as usize..e as usize]) {
                    caller.data().lock().unwrap().events.push(event);
                }
            }
        }
    })?;

    linker.func_wrap(module, "log_utf8", |mut caller: wasmtime::Caller<'_, Ctx>,
        msg_ptr: u64, msg_len: u64|
    {
        let mem = caller.get_export("memory").and_then(|e| e.into_memory());
        if let Some(mem) = mem {
            let data = mem.data(&caller);
            let end = msg_ptr.checked_add(msg_len).filter(|&e| e as usize <= data.len());
            if let Some(e) = end {
                let msg = String::from_utf8_lossy(&data[msg_ptr as usize..e as usize]).to_string();
                caller.data().lock().unwrap().logs.push(msg);
            }
        }
    })?;

    linker.func_wrap(module, "panic_utf8", |mut caller: wasmtime::Caller<'_, Ctx>,
        msg_ptr: u64, msg_len: u64| -> Result<()>
    {
        let mem = caller.get_export("memory").and_then(|e| e.into_memory());
        let msg = if let Some(mem) = mem {
            let data = mem.data(&caller);
            let end = msg_ptr.checked_add(msg_len).filter(|&e| e as usize <= data.len());
            if let Some(e) = end {
                String::from_utf8_lossy(&data[msg_ptr as usize..e as usize]).to_string()
            } else {
                "contract panicked (out-of-bounds message pointer)".to_string()
            }
        } else {
            "contract panicked".to_string()
        };
        caller.data().lock().unwrap().panic_message = Some(msg.clone());
        Err(anyhow::anyhow!("{}", msg))
    })?;

    // ── Balance / Transfer ────────────────────────────────────────────────────
    // Balances cross the WASM boundary as u64 (satoshis). Internal state uses u128.

    linker.func_wrap(module, "balance_of", |mut caller: wasmtime::Caller<'_, Ctx>,
        account_ptr: u64, account_len: u64| -> u64
    {
        let mem = caller.get_export("memory").and_then(|e| e.into_memory());
        if let Some(mem) = mem {
            let data = mem.data(&caller);
            let end = account_ptr.checked_add(account_len).filter(|&e| e as usize <= data.len());
            if let Some(e) = end {
                let account = String::from_utf8_lossy(&data[account_ptr as usize..e as usize]).to_string();
                let ctx = caller.data().lock().unwrap();
                let bal = *ctx.balances.get(&account).unwrap_or(&0);
                bal.min(u64::MAX as u128) as u64
            } else { 0 }
        } else { 0 }
    })?;

    linker.func_wrap(module, "transfer", |mut caller: wasmtime::Caller<'_, Ctx>,
        to_ptr: u64, to_len: u64, amount: u64| -> u64
    {
        let mem = caller.get_export("memory").and_then(|e| e.into_memory());
        if let Some(mem) = mem {
            let data = mem.data(&caller);
            let end = to_ptr.checked_add(to_len).filter(|&e| e as usize <= data.len());
            let end = match end { Some(e) => e, None => return 0 };
            let to = String::from_utf8_lossy(&data[to_ptr as usize..end as usize])
                .to_string();
            let mut ctx = caller.data().lock().unwrap();
            let contract_id = ctx.contract_id.clone();
            let contract_balance = *ctx.balances.get(&contract_id).unwrap_or(&0);
            let available = contract_balance.saturating_sub(ctx.reserved_balance);
            if available >= amount as u128 {
                ctx.reserved_balance += amount as u128;
                ctx.pending_transfers.push((to, amount as u128));
                1
            } else { 0 }
        } else { 0 }
    })?;

    // Stub cross-contract promise (Phase 2 — returns 0 for now)
    linker.func_wrap(module, "promise_create", |_caller: wasmtime::Caller<'_, Ctx>,
        _: u64, _: u64, _: u64, _: u64, _: u64, _: u64, _: u64, _: u64| -> u64 { 0 })?;

    Ok(())
}

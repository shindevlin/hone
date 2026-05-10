//! Host functions provided by the BTCPC runtime to executing contracts.
//!
//! On-chain (wasm32): these are extern "C" imports from the runtime.
//! Off-chain (tests): a mock implementation is injected.

#[cfg(target_arch = "wasm32")]
use alloc::{string::String, vec, vec::Vec};
#[cfg(not(target_arch = "wasm32"))]
use std::string::String;

use crate::types::{AccountId, Balance, Epoch};

// ── WASM host function imports ──────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
pub(crate) mod sys {
    extern "C" {
        // Identity
        pub fn env_signer(register_id: u64);
        pub fn env_predecessor(register_id: u64);
        pub fn env_contract_id(register_id: u64);

        // Chain state
        pub fn env_epoch() -> u64;
        // Returns attached deposit in dreams, capped at u64::MAX.
        pub fn env_attached_deposit() -> u64;

        // Storage
        pub fn storage_write(key_ptr: u64, key_len: u64, val_ptr: u64, val_len: u64) -> u64;
        pub fn storage_read(key_ptr: u64, key_len: u64, register_id: u64) -> u64;
        pub fn storage_remove(key_ptr: u64, key_len: u64, register_id: u64) -> u64;
        pub fn storage_has_key(key_ptr: u64, key_len: u64) -> u64;

        // Balance / transfer — amounts in dreams (u64, max ~18.4 × 10^18).
        pub fn balance_of(account_ptr: u64, account_len: u64) -> u64;
        pub fn transfer(to_ptr: u64, to_len: u64, amount: u64) -> u64;

        // Registers (for returning variable-length data from host to contract)
        pub fn read_register(register_id: u64, ptr: u64);
        pub fn register_len(register_id: u64) -> u64;

        // Output
        pub fn value_return(value_ptr: u64, value_len: u64);
        pub fn emit_event(event_ptr: u64, event_len: u64);
        pub fn log_utf8(msg_ptr: u64, msg_len: u64);
        pub fn panic_utf8(msg_ptr: u64, msg_len: u64) -> !;

        // Cross-contract calls (stub: returns 0 until Phase 2 implementation)
        pub fn promise_create(
            contract_ptr: u64, contract_len: u64,
            method_ptr: u64, method_len: u64,
            args_ptr: u64, args_len: u64,
            amount: u64,
            gas: u64,
        ) -> u64;
    }
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Returns the account ID of the transaction signer.
pub fn signer() -> AccountId {
    #[cfg(target_arch = "wasm32")]
    {
        unsafe {
            sys::env_signer(0);
            read_register_str(0)
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::mock::env::signer()
    }
}

/// Returns the immediate caller (for cross-contract calls, differs from signer).
pub fn predecessor() -> AccountId {
    #[cfg(target_arch = "wasm32")]
    {
        unsafe {
            sys::env_predecessor(0);
            read_register_str(0)
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::mock::env::predecessor()
    }
}

/// Returns this contract's own address.
pub fn current_contract() -> AccountId {
    #[cfg(target_arch = "wasm32")]
    {
        unsafe {
            sys::env_contract_id(0);
            read_register_str(0)
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::mock::env::current_contract()
    }
}

/// Returns the attached deposit in dreams.
pub fn attached_deposit() -> Balance {
    #[cfg(target_arch = "wasm32")]
    unsafe { sys::env_attached_deposit() as u128 }
    #[cfg(not(target_arch = "wasm32"))]
    crate::mock::env::attached_deposit()
}

/// Returns the current chain epoch number.
pub fn epoch() -> Epoch {
    #[cfg(target_arch = "wasm32")]
    unsafe { sys::env_epoch() }
    #[cfg(not(target_arch = "wasm32"))]
    crate::mock::env::epoch()
}

/// BTCPC balance of `account` in dreams (1 BTCPC = 10_000_000_000 dreams).
pub fn balance_of(account: &AccountId) -> Balance {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        let bytes = account.as_bytes();
        sys::balance_of(bytes.as_ptr() as u64, bytes.len() as u64) as u128
    }
    #[cfg(not(target_arch = "wasm32"))]
    crate::mock::env::balance_of(account)
}

/// Transfer `amount` dreams from the contract's own balance to `to`.
/// Returns true on success; returns false if insufficient funds.
/// Amount is capped at u64::MAX dreams; amounts above that return false.
pub fn transfer(to: &AccountId, amount: Balance) -> bool {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        if amount > u64::MAX as u128 { return false; }
        let bytes = to.as_bytes();
        sys::transfer(bytes.as_ptr() as u64, bytes.len() as u64, amount as u64) != 0
    }
    #[cfg(not(target_arch = "wasm32"))]
    crate::mock::env::transfer(to, amount)
}

/// Emit a log message (visible in receipts, not stored in state).
pub fn log(msg: &str) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        let bytes = msg.as_bytes();
        sys::log_utf8(bytes.as_ptr() as u64, bytes.len() as u64);
    }
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("[CONTRACT LOG] {}", msg);
}

/// Emit a structured event payload.
pub fn emit_event(payload: &[u8]) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        sys::emit_event(payload.as_ptr() as u64, payload.len() as u64);
    }
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("[CONTRACT EVENT] {}", String::from_utf8_lossy(payload));
}

/// Panic with a message; all state changes in this call are rolled back.
pub fn panic_str(msg: &str) -> ! {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        let bytes = msg.as_bytes();
        sys::panic_utf8(bytes.as_ptr() as u64, bytes.len() as u64)
    }
    #[cfg(not(target_arch = "wasm32"))]
    panic!("{}", msg)
}

/// Read the method name from the dispatch register (register 0).
/// Called by macro-generated dispatch functions.
pub fn read_method_name() -> String {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        let len = sys::register_len(0) as usize;
        let mut buf = vec![0u8; len];
        sys::read_register(0, buf.as_mut_ptr() as u64);
        String::from_utf8(buf).unwrap_or_default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    crate::mock::env::method_name()
}

// ── Storage (low-level, use collections for ergonomics) ─────────────────────

pub fn storage_write(key: &[u8], value: &[u8]) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        sys::storage_write(
            key.as_ptr() as u64, key.len() as u64,
            value.as_ptr() as u64, value.len() as u64,
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    crate::mock::storage::write(key, value);
}

pub fn storage_read(key: &[u8]) -> Option<Vec<u8>> {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        if sys::storage_read(key.as_ptr() as u64, key.len() as u64, 1) == 1 {
            let len = sys::register_len(1) as usize;
            let mut buf = vec![0u8; len];
            sys::read_register(1, buf.as_mut_ptr() as u64);
            Some(buf)
        } else {
            None
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    crate::mock::storage::read(key)
}

pub fn storage_remove(key: &[u8]) -> bool {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        sys::storage_remove(key.as_ptr() as u64, key.len() as u64, 0) == 1
    }
    #[cfg(not(target_arch = "wasm32"))]
    crate::mock::storage::remove(key)
}

pub fn storage_has_key(key: &[u8]) -> bool {
    #[cfg(target_arch = "wasm32")]
    unsafe { sys::storage_has_key(key.as_ptr() as u64, key.len() as u64) == 1 }
    #[cfg(not(target_arch = "wasm32"))]
    crate::mock::storage::has_key(key)
}

// Returns the call input bytes (JSON-encoded arguments).
pub fn input() -> Vec<u8> {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        let len = sys::register_len(u64::MAX) as usize;
        if len == u64::MAX as usize { return vec![]; }
        let mut buf = vec![0u8; len];
        sys::read_register(u64::MAX, buf.as_mut_ptr() as u64);
        buf
    }
    #[cfg(not(target_arch = "wasm32"))]
    crate::mock::env::input()
}

/// Write the return value for a `#[call]` or `#[view]` method.
pub fn value_return(value: &[u8]) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        sys::value_return(value.as_ptr() as u64, value.len() as u64);
    }
    #[cfg(not(target_arch = "wasm32"))]
    crate::mock::env::set_return_value(value.to_vec());
}

// ── Internal helpers ─────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
unsafe fn read_register_str(register_id: u64) -> String {
    let len = sys::register_len(register_id) as usize;
    let mut buf = vec![0u8; len];
    sys::read_register(register_id, buf.as_mut_ptr() as u64);
    String::from_utf8(buf).unwrap_or_default()
}

// ── Promise API (stub — Phase 2) ──────────────────────────────────────────────

/// Create a cross-contract call stub. Currently returns 0 (not yet implemented).
/// Amount is in dreams and capped at u64::MAX.
pub fn promise_create(
    contract: &str,
    method: &str,
    args: &[u8],
    amount: Balance,
    gas: u64,
) -> u64 {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        let contract_bytes = contract.as_bytes();
        let method_bytes = method.as_bytes();
        sys::promise_create(
            contract_bytes.as_ptr() as u64, contract_bytes.len() as u64,
            method_bytes.as_ptr() as u64, method_bytes.len() as u64,
            args.as_ptr() as u64, args.len() as u64,
            amount.min(u64::MAX as u128) as u64,
            gas,
        )
    }
    #[cfg(not(target_arch = "wasm32"))]
    { let _ = (contract, method, args, amount, gas); 0 }
}

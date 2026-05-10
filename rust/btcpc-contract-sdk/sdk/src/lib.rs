/*!
# BTCPC Contract SDK

Write smart contracts for the BTCPC chain in Rust. Contracts compile to
`wasm32-unknown-unknown` and are stored and executed on-chain.

## Quick Start

```rust
use btcpc_contract_sdk::*;

#[btcpc_contract]
pub struct Counter {
    count: u64,
}

#[btcpc_impl]
impl Counter {
    #[init]
    pub fn new(start: u64) -> Self {
        Self { count: start }
    }

    #[call]
    pub fn increment(&mut self) {
        self.count += 1;
    }

    #[view]
    pub fn get(&self) -> u64 {
        self.count
    }
}
```
*/

#![cfg_attr(target_arch = "wasm32", no_std)]

// Make alloc types available in no_std mode (wasm32 target).
#[cfg(target_arch = "wasm32")]
extern crate alloc;

#[cfg(feature = "wee_alloc")]
#[cfg(target_arch = "wasm32")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

pub mod env;
pub mod storage;
pub mod types;
pub mod event;
pub mod promise;
pub mod collections;
#[cfg(not(target_arch = "wasm32"))]
pub mod mock;

pub use btcpc_contract_sdk_macros::{btcpc_contract, btcpc_impl, init, call, view, private, callback};

pub use types::{AccountId, Balance, Epoch, ContractId};
pub use storage::{StorageKey, LookupMap, UnorderedMap, Vector, LazyOption};
pub use env::*;

// Re-export borsh for contract state serialization
pub use borsh::{BorshDeserialize, BorshSerialize};
pub use serde::{Deserialize, Serialize};
pub use serde_json;

/// Private re-exports for macro-generated code. Not part of the public API.
#[doc(hidden)]
pub mod __private {
    #[cfg(target_arch = "wasm32")]
    pub use alloc::{vec, vec::Vec, format};
    #[cfg(not(target_arch = "wasm32"))]
    pub use std::{vec, vec::Vec, format};
}

/// Panic the contract with a message. State changes are rolled back.
#[macro_export]
macro_rules! require {
    ($cond:expr, $msg:literal) => {
        if !($cond) {
            $crate::env::panic_str($msg);
        }
    };
    ($cond:expr, $msg:expr) => {
        if !($cond) {
            $crate::env::panic_str(&$msg);
        }
    };
}

/// Log a message (visible in transaction receipts, not stored in state).
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::env::log(&format!($($arg)*));
    };
}

/// Emit a structured event (indexed, queryable).
#[macro_export]
macro_rules! emit {
    ($event:expr) => {
        $crate::event::emit_event(&$event);
    };
}

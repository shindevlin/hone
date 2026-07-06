//! Cross-contract call API.

#[cfg(target_arch = "wasm32")]
extern crate alloc;
#[cfg(target_arch = "wasm32")]
use alloc::{string::{String, ToString}, vec::Vec, borrow::ToOwned};

use crate::types::{Balance, Gas};

/// A deferred cross-contract call.
pub struct Promise {
    #[allow(dead_code)]
    id: u64,
}

impl Promise {
    /// Call a method on another contract.
    pub fn new(contract: impl AsRef<str>) -> PromiseBuilder {
        PromiseBuilder { contract: contract.as_ref().to_owned() }
    }
}

pub struct PromiseBuilder {
    contract: String,
}

impl PromiseBuilder {
    pub fn call(
        self,
        method: impl AsRef<str>,
        args: serde_json::Value,
        deposit: Balance,
        gas: Gas,
    ) -> Promise {
        let args_bytes = serde_json::to_vec(&args).unwrap_or_default();
        let id = crate::env::promise_create(
            &self.contract,
            method.as_ref(),
            &args_bytes,
            deposit,
            gas,
        );
        Promise { id }
    }
}

/// Suggested gas amounts for common patterns.
pub mod gas {
    use super::Gas;
    pub const BASIC: Gas = 5_000_000_000;
    pub const TRANSFER: Gas = 10_000_000_000;
    pub const COMPLEX: Gas = 50_000_000_000;
    pub const CALLBACK: Gas = 10_000_000_000;
}

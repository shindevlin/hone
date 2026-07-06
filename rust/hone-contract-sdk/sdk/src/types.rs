#[cfg(target_arch = "wasm32")]
extern crate alloc;
#[cfg(target_arch = "wasm32")]
use alloc::{string::String, vec::Vec};

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// A HONE account identifier (username or honead… address).
pub type AccountId = String;

/// A contract address (honesc… prefix).
pub type ContractId = String;

/// Balance in hunits (1 HONE = 10_000_000_000 hunits).
pub type Balance = u128;

/// Chain epoch number (one epoch = 30 seconds).
pub type Epoch = u64;

/// Gas for metered execution within a single call.
pub type Gas = u64;

/// 1 full HONE expressed in hunits (10^10).
pub const HUNITS_PER_HONE: Balance = 10_000_000_000;

#[deprecated = "use HUNITS_PER_HONE"]
pub const ONE_HONE: Balance = HUNITS_PER_HONE;

/// Maximum gas per contract call.
pub const MAX_GAS: Gas = 300_000_000_000;

/// Represents a storage key prefix for namespacing collections.
#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Clone, Debug)]
pub struct StoragePrefix(pub Vec<u8>);

impl StoragePrefix {
    pub fn new(prefix: &[u8]) -> Self {
        Self(prefix.to_vec())
    }
}

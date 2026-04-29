use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// A BTCPC account identifier (username or BTCPCad… address).
pub type AccountId = String;

/// A contract address (BTCPCsc… prefix).
pub type ContractId = String;

/// Balance in dreams (1 BTCPC = 10_000_000_000 dreams).
pub type Balance = u128;

/// Chain epoch number (one epoch = 30 seconds).
pub type Epoch = u64;

/// Gas for metered execution within a single call.
pub type Gas = u64;

/// 1 full BTCPC expressed in dreams (10^10).
pub const DREAMS_PER_BTCPC: Balance = 10_000_000_000;

#[deprecated = "use DREAMS_PER_BTCPC"]
pub const ONE_BTCPC: Balance = DREAMS_PER_BTCPC;

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

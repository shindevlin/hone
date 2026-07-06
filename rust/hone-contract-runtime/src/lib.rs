/*!
# HONE Contract Runtime

Executes WASM smart contracts compiled with `hone-contract-sdk`.

## Execution model

Each call to [`execute_call`] is fully isolated:
- Fresh WASM instance per call (no shared mutable state between calls)
- Contract state loaded from the host KV store before execution
- Writes buffered; only committed to host store if execution succeeds
- Gas metering via wasmtime fuel (1 fuel unit ≈ 1 WASM instruction)
- All host functions are deterministic given the same chain state

## Integration

The runtime is a library embedded in the `hone-p2p` Rust sidecar.
Node.js invokes it via the Unix socket IPC using the `contract_call`
and `contract_deploy` JSON-RPC methods.
*/

pub mod host;
pub mod executor;
pub mod state;
pub mod gas;
pub mod error;

pub use executor::{execute_call, execute_deploy, execute_view};
pub use state::{ContractState, StorageKv};
pub use error::RuntimeError;

use serde::{Deserialize, Serialize};

/// Input to a contract call (from Node.js via IPC).
#[derive(Debug, Serialize, Deserialize)]
pub struct CallRequest {
    /// Contract address (honesc…)
    pub contract_id: String,
    /// Method name
    pub method: String,
    /// JSON-encoded arguments
    pub args: serde_json::Value,
    /// Calling account
    pub signer: String,
    /// Immediate caller (same as signer unless cross-contract)
    pub predecessor: String,
    /// HONE satoshis attached to this call
    pub attached_deposit: u128,
    /// Gas limit (fuel units)
    pub gas: u64,
    /// Current chain epoch
    pub epoch: u64,
}

/// Output from a contract call.
#[derive(Debug, Serialize, Deserialize)]
pub struct CallResult {
    /// Whether execution succeeded
    pub success: bool,
    /// Return value (JSON), if any
    pub result: Option<serde_json::Value>,
    /// Gas consumed
    pub gas_used: u64,
    /// State changes to commit (empty on failure)
    pub storage_writes: Vec<(String, Vec<u8>)>,
    /// Keys to delete from state (empty on failure)
    pub storage_deletes: Vec<String>,
    /// Pending token transfers requested by the contract (to, amount_satoshis).
    /// Only populated on success.
    pub pending_transfers: Vec<(String, u128)>,
    /// Events emitted
    pub events: Vec<serde_json::Value>,
    /// Log messages
    pub logs: Vec<String>,
    /// Error message on failure
    pub error: Option<String>,
}

/// Input for deploying a new contract.
#[derive(Debug, Serialize, Deserialize)]
pub struct DeployRequest {
    /// Deploying account
    pub deployer: String,
    /// WASM bytecode (base64)
    pub wasm_b64: String,
    /// Constructor method name (default: "new")
    pub init_method: Option<String>,
    /// Constructor arguments (JSON)
    pub init_args: Option<serde_json::Value>,
    /// Gas limit
    pub gas: u64,
    /// Chain epoch
    pub epoch: u64,
    /// Nonce for address derivation
    pub nonce: u64,
}

/// Derive a deterministic contract address.
pub fn derive_contract_address(deployer: &str, epoch: u64, nonce: u64) -> String {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(deployer.as_bytes());
    hasher.update(epoch.to_le_bytes());
    hasher.update(nonce.to_le_bytes());
    let hash = hex::encode(hasher.finalize());
    format!("honesc{}", &hash[..36])
}

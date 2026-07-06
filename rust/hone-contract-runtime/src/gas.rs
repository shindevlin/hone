//! Gas metering constants.
//! 1 EB = 1_000_000 fuel units. Contracts pay EB at call submission;
//! the runtime enforces the gas limit via wasmtime's fuel API.

/// Fuel units per EB unit (1 EB ≈ 1 million WASM instructions).
pub const FUEL_PER_EB: u64 = 1_000_000;

/// Base gas cost for any contract call (covers dispatch overhead).
pub const BASE_CALL_FUEL: u64 = 500_000;

/// Extra fuel cost per byte of argument input.
pub const FUEL_PER_INPUT_BYTE: u64 = 10;

/// Extra fuel cost per storage read.
pub const FUEL_PER_STORAGE_READ: u64 = 50_000;

/// Extra fuel cost per storage write (covers state serialization).
pub const FUEL_PER_STORAGE_WRITE: u64 = 100_000;

/// Maximum fuel for a single contract call (300 EB equivalent).
pub const MAX_CALL_FUEL: u64 = 300 * FUEL_PER_EB;

/// EB cost for a `#[view]` call (metered but cheaper).
pub const VIEW_EB_COST: u64 = 5;

/// EB cost for a `#[call]` method (base; actual EB = gas_used / FUEL_PER_EB).
pub const CALL_EB_BASE: u64 = 50;

/// EB cost to deploy a contract (base; +1 EB per 1024 bytes of WASM).
pub const DEPLOY_EB_BASE: u64 = 500;

pub const fn deploy_eb_cost(wasm_bytes: usize) -> u64 {
    DEPLOY_EB_BASE + (wasm_bytes as u64 / 1024)
}

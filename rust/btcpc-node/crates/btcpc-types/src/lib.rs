pub mod account;
pub mod block;
pub mod emission;
pub mod entry;

pub use account::*;
pub use block::*;
pub use emission::*;
pub use entry::*;

/// 1 BTCPC = 10_000_000_000 dreams (10^10)
pub const DREAMS_PER_BTCPC: u64 = 10_000_000_000;
#[deprecated = "use DREAMS_PER_BTCPC"]
pub const SATOSHIS_PER_BTCPC: u64 = DREAMS_PER_BTCPC;
/// Native token symbol
pub const NATIVE_TOKEN: &str = "BTCPC";
/// Initial epoch duration in milliseconds (era 0).  Use epoch_duration_ms(epoch) for the actual value.
pub const EPOCH_MS: u64 = 30_000;

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

/// Chain identifiers — used in canonical signing messages and gossip validation.
pub const MAINNET_CHAIN_ID: &str = "btcpc-1";
pub const TESTNET_CHAIN_ID: &str = "btcpc-satoshi";

/// Name registration stake: locked in the account on creation, released only if the
/// account is dissolved. Prevents squatting. = 10 BTCPC.
pub const NAME_REGISTRATION_STAKE: u64 = 10 * DREAMS_PER_BTCPC;

/// Accounts that are exempt from paying the name registration stake (genesis operators,
/// shindevlin's reserved namespace). These are created at genesis or via GenesisAlloc.
pub const STAKE_EXEMPT_ACCOUNTS: &[&str] = &["shindevlin", "__testnet_fund__", "__recycle__", "treasury"];

// ── Liveness / dead-man's-switch ─────────────────────────────────────────────

/// Epochs of silence before the liveness countdown begins.
/// 30 s/epoch × 1_051_920 epochs ≈ 1 year.  Grace = 3 years.
pub const LIVENESS_GRACE_EPOCHS: u64 = 3 * 1_051_920;

/// After grace + LIVENESS_DECAY_DELAY epochs of additional silence, token decay begins.
/// 2 more years = 5 total years of silence before any bleed.
pub const LIVENESS_DECAY_DELAY_EPOCHS: u64 = 2 * 1_051_920;

/// Half-life in epochs.  Tokens halve every 2 years once decay starts.
/// Applied at each finalization boundary (every 100 epochs).
pub const LIVENESS_HALF_LIFE_EPOCHS: u64 = 2 * 1_051_920;

pub mod account;
pub mod block;
pub mod emission;
pub mod entry;

pub use account::*;
pub use block::*;
pub use emission::*;
pub use entry::*;

/// 1 HONE = 10_000_000_000 hunits (10^10). hunit = the smallest unit (H-units).
pub const HUNITS_PER_HONE: u64 = 10_000_000_000;
#[deprecated = "use HUNITS_PER_HONE"]
pub const SATOSHIS_PER_HONE: u64 = HUNITS_PER_HONE;
/// Native token symbol
pub const NATIVE_TOKEN: &str = "HONE";
/// Initial epoch duration in milliseconds (era 0).  Use epoch_duration_ms(epoch) for the actual value.
pub const EPOCH_MS: u64 = 30_000;

/// Chain identifiers — used in canonical signing messages and gossip validation.
pub const MAINNET_CHAIN_ID: &str = "hone";
pub const TESTNET_CHAIN_ID: &str = "hone-testnet";

/// Name registration stake: locked in the account on creation, released only if the
/// account is dissolved. Prevents squatting. = 10 HONE.
pub const NAME_REGISTRATION_STAKE: u64 = 10 * HUNITS_PER_HONE;

/// Accounts that are exempt from paying the name registration stake (genesis operators,
/// shindevlin's reserved namespace). These are created at genesis or via GenesisAlloc.
pub const STAKE_EXEMPT_ACCOUNTS: &[&str] = &["shindevlin", "__testnet_fund__", "__recycle__", "treasury"];

/// Clock bootstrap grace window: the last epoch (inclusive, counted from genesis
/// epoch 0) during which clock nodes may register at zero stake and auto-accumulate
/// their `clock_min_stake` from ClockReward instead of paying it up front.
///
/// HONE funds no account at genesis (pure POW), so at launch nobody can post the
/// clock minimum stake — without this window, no clock could register, no epoch
/// could seal, and the chain would deadlock at block 0. During the first 100,000
/// epochs (≈34.7 days at 30 s/epoch, "the first 100k blocks") `ClockNodeRegister`
/// is accepted at stake 0 with no debit, and each `ClockReward` earned by an
/// under-staked clock is routed into its stake (capped at the minimum) until the
/// minimum is reached. After this epoch, the normal minimum-stake rule applies and
/// the auto-build stops. Consensus-critical: every node MUST use this exact value.
/// See docs/CLOCK_BOOTSTRAP_GRACE.md.
pub const CLOCK_BOOTSTRAP_GRACE_END_EPOCH: u64 = 100_000;

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

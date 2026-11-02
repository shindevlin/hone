pub mod account;
pub mod block;
pub mod emission;
pub mod entry;
pub mod capabilities;

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
// Extended 100_000 -> 250_000 (Shin, 2026-07-30). At 30s epochs, 100k is ~35 days
// from genesis and the chain was already 25 days in when it first sealed, leaving
// ~10 days of grace. 250k is ~87 days, giving operators time to accumulate balance
// and stake before fees and the stake gate take effect.
pub const CLOCK_BOOTSTRAP_GRACE_END_EPOCH: u64 = 250_000;

/// Signed-authorisation fix (security). Before this epoch, `check_signature` SKIPPED
/// verification when the signer had no key registered for the required role (e.g. no
/// "active" key — true for nearly every account), and when the signer had no account
/// record at all (e.g. protocol funds). That let anyone submit an UNSIGNED Transfer,
/// stake, etc. for such accounts. From this epoch: a missing role key falls back to the
/// account's posting key, and a signer with no usable key (or no account) is rejected.
/// The owner-role bootstrap for a keyless account (first AccountUpdateKey) is unchanged.
/// Consensus-critical (tx acceptance): every node MUST use this exact value.
pub const SIGNED_AUTH_ACTIVATION_EPOCH: u64 = 320_200;

/// Device clocks: one account, many devices, each staked separately.
///
/// A device clock seat is the node id `"{owner}@{pubkey}"`, where `pubkey` is the
/// device's 32-byte ed25519 key as 64 lowercase hex chars. The id binds the key, so
/// a device seal is only valid when signed by that key, and nobody else can occupy
/// the seat. The seat counts toward clock quorum when the OWNER has staked at least
/// `clock_min_stake` on it (NodeRoleStake, node = the device id); only the owner may
/// stake on it. Clock rewards earned by the seat are credited to the owner account.
///
/// Consensus-critical: device seats are ignored (not registered, no rewards) before
/// this epoch, so every node must be upgraded before it. Every node MUST use this
/// exact value.
pub const DEVICE_CLOCK_ACTIVATION_EPOCH: u64 = 321_000;

/// Split a device clock id `"{owner}@{pubkey_hex}"` into `(owner, pubkey_hex)`.
/// Returns None for plain account ids or anything malformed.
pub fn parse_device_clock_id(id: &str) -> Option<(&str, &str)> {
    let (owner, pk) = id.split_once('@')?;
    if owner.is_empty() || owner.contains(':') {
        return None;
    }
    if pk.len() != 64 || !pk.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    Some((owner, pk))
}

/// Confirmation depth before an epoch's reward/recycle/decay mutation is applied.
///
/// BUG 6: the winning EpochFinalize entry drives the balance mutation, but a
/// heavier-quorum competitor can still overwrite fork choice for a few epochs after
/// the first entry lands. Applying the mutation on first-seen would credit off a
/// possibly-losing entry. So we only apply an epoch's rewards once it is this many
/// epochs behind the current tip — long enough that the fork-choice rewrite window
/// has closed. Reuses the same finality tolerance as HIVE_REPLICA_MIN_CONFIRMATIONS
/// (20); at 30 s/epoch that is ~10 minutes, far wider than any real rewrite window.
pub const REWARD_FINALITY_DEPTH: u64 = 20;

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


// ── Memory service signing ──────────────────────────────────────────────────────
/// From this entry epoch on, MemorySet / MemoryDelete are signed over an explicit
/// client-reproducible message (account, chain_id, key, nonce, type[, value]) instead of
/// the generic full-entry JSON (which contained the server-set epoch and the signature
/// itself, so no client could sign it). Consensus-critical: every node must run a binary
/// with this value before it is reached.
pub const MEMORY_SIGNING_ACTIVATION_EPOCH: u64 = 349_100;

// ── Identity NFT (names as transferable assets) ─────────────────────────────────
/// Identity-NFT entries (IdentityNft*) are rejected before this epoch. Consensus-
/// critical: every node MUST use this exact value and be upgraded before it.
/// PLACEHOLDER — confirm against the upgrade rollout before activation.
pub const IDENTITY_NFT_ACTIVATION_EPOCH: u64 = 333_333;

/// Founder set for the permalock 2-of-3 multisig that can lift a permanent soulbound.
/// PLACEHOLDER — confirm the exact three founder accounts.
pub const IDENTITY_FOUNDER_ACCOUNTS: &[&str] = &["shindevlin", "josh", "natoshisakamoto"];

/// Distinct founder signatures required to lift a permalock.
pub const IDENTITY_FOUNDER_THRESHOLD: usize = 2;

/// Registrations after which free open-mint ends and new names must go to auction.
pub const IDENTITY_OPEN_PHASE_LIMIT: u64 = 1_000;

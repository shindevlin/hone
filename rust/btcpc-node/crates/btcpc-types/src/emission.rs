//! BTCPC emission schedule — epoch-time doubling model.
//!
//! Instead of halving per-epoch reward (Bitcoin model), BTCPC doubles the epoch
//! duration every DOUBLING_INTERVAL epochs.  The effect on daily emission is
//! identical (half as many epochs per day = half the daily reward), but the
//! per-epoch reward stays constant throughout the new-supply phase.
//!
//! # Eras
//!
//! | Era | Epoch time | Daily new-supply emission | Duration  |
//! |-----|-----------|---------------------------|-----------|
//! |  0  | 30s       | 5,760 BTCPC/day           | ~4 years  |
//! |  1  | 60s       | 2,880 BTCPC/day           | ~8 years  |
//! |  2  | 2 min     | 1,440 BTCPC/day           | ~16 years |
//! |  3  | 4 min     | 720 BTCPC/day             | ~32 years |
//! |  4  | 8 min     | 360 BTCPC/day             | ~64 years |
//! |  5+ | 16 min    | 0 (recycled tokens only)  | ∞         |
//!
//! Total new supply exhausted after ~124 years (2026 → ~2150, analogous to
//! Bitcoin's ~2140 endpoint).  Era 5 runs forever: miners receive the recycled
//! fraction of all reward distributions (see `split_reward` in chain-core).
//!
//! # Blob capacity
//!
//! Blobs carry a `fee: Dreams` field, so spam is economically constrained.
//! There is no hard per-epoch blob count cap.  Daily blob throughput stays
//! constant because miners accumulate more entries per longer epoch.

/// Epochs per era before epoch duration doubles.
pub const DOUBLING_INTERVAL: u64 = 4_200_000;

/// Initial epoch duration in milliseconds (era 0: 30 seconds).
pub const INITIAL_EPOCH_MS: u64 = 30_000;

/// Per-epoch block reward in dreams during the new-supply phase (eras 0-4).
/// 2 BTCPC × 10^10 dreams/BTCPC = 20_000_000_000 dreams.
pub const BLOCK_REWARD_DREAMS: u64 = 2 * 10_000_000_000;

/// Total new-supply cap in dreams.
/// 42_000_000 BTCPC × 10^10 dreams/BTCPC.
pub const SUPPLY_CAP_DREAMS: u64 = 42_000_000 * 10_000_000_000;

/// First era whose rewards come entirely from recycled tokens (no new supply).
pub const RECYCLE_ERA: u64 = 5;

/// Which era a given epoch falls in (0-indexed).
#[inline]
pub fn era(epoch: u64) -> u64 {
    epoch / DOUBLING_INTERVAL
}

/// Epoch duration in milliseconds at a given epoch.
///
/// Doubles every DOUBLING_INTERVAL epochs, capped at era 5 (16 minutes).
/// Era 5+ runs forever at 16-minute intervals.
#[inline]
pub fn epoch_duration_ms(epoch: u64) -> u64 {
    let shift = era(epoch).min(RECYCLE_ERA);
    INITIAL_EPOCH_MS << shift
}

/// Block reward in dreams for the given epoch number.
///
/// Returns `BLOCK_REWARD_DREAMS` until the supply cap would be exceeded, then
/// returns the remaining dreams (may be less than a full reward), then 0 for
/// all epochs in era 5+.  Era 5 rewards are distributed from the recycled-token
/// fund managed separately.
pub fn block_reward_at(epoch: u64) -> u64 {
    if era(epoch) >= RECYCLE_ERA {
        return 0; // recycled-token era — separate reward path
    }
    // Epochs start at 1; epoch 0 is the genesis marker with no reward.
    let total_already_emitted = epoch.saturating_mul(BLOCK_REWARD_DREAMS);
    if total_already_emitted >= SUPPLY_CAP_DREAMS {
        return 0;
    }
    let remaining = SUPPLY_CAP_DREAMS - total_already_emitted;
    BLOCK_REWARD_DREAMS.min(remaining)
}

/// Wall-clock offset in milliseconds from genesis for the start of `epoch`.
///
/// Not relative to Unix epoch — add genesis_timestamp_ms to get absolute time.
pub fn epoch_start_ms_from_genesis(epoch: u64) -> u64 {
    if epoch == 0 {
        return 0;
    }
    let e = era(epoch);
    // Sum of all fully-completed era durations
    let full_era_ms: u64 = (0..e).map(|k| {
        let era_epoch_ms = INITIAL_EPOCH_MS << k.min(RECYCLE_ERA);
        DOUBLING_INTERVAL * era_epoch_ms
    }).sum();
    // Remaining epochs within the current era
    let epoch_in_era = epoch - e * DOUBLING_INTERVAL;
    let cur_epoch_ms = INITIAL_EPOCH_MS << e.min(RECYCLE_ERA);
    full_era_ms + epoch_in_era * cur_epoch_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn era_boundaries() {
        assert_eq!(era(0), 0);
        assert_eq!(era(DOUBLING_INTERVAL - 1), 0);
        assert_eq!(era(DOUBLING_INTERVAL), 1);
        assert_eq!(era(RECYCLE_ERA * DOUBLING_INTERVAL), RECYCLE_ERA);
    }

    #[test]
    fn epoch_durations() {
        assert_eq!(epoch_duration_ms(0), 30_000);
        assert_eq!(epoch_duration_ms(DOUBLING_INTERVAL), 60_000);
        assert_eq!(epoch_duration_ms(2 * DOUBLING_INTERVAL), 120_000);
        assert_eq!(epoch_duration_ms(4 * DOUBLING_INTERVAL), 480_000);
        // Era 5+: capped at 16 minutes
        assert_eq!(epoch_duration_ms(5 * DOUBLING_INTERVAL), 960_000);
        assert_eq!(epoch_duration_ms(100 * DOUBLING_INTERVAL), 960_000);
    }

    #[test]
    fn supply_exhausted_after_5_eras() {
        // Total new-supply epochs: 5 eras × 4_200_000 = 21_000_000
        let last_rewarded = 5 * DOUBLING_INTERVAL - 1;
        assert!(block_reward_at(last_rewarded) > 0);
        // Era 5 onward: no new supply
        assert_eq!(block_reward_at(5 * DOUBLING_INTERVAL), 0);
        // Well within era 0: full reward
        assert_eq!(block_reward_at(1), BLOCK_REWARD_DREAMS);
    }

    #[test]
    fn total_supply_correct() {
        // Sum reward for all 21_000_000 new-supply epochs
        let total: u64 = (0..5 * DOUBLING_INTERVAL)
            .map(block_reward_at)
            .sum();
        assert_eq!(total, SUPPLY_CAP_DREAMS);
    }

    #[test]
    fn genesis_to_cap_duration() {
        let cap_epoch = 5 * DOUBLING_INTERVAL;
        let ms = epoch_start_ms_from_genesis(cap_epoch);
        let years = ms as f64 / 1000.0 / 86400.0 / 365.25;
        // Should be ~124 years
        assert!((120.0..130.0).contains(&years), "expected ~124 years, got {:.1}", years);
    }
}

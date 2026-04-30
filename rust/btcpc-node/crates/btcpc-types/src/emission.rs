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
//! Bitcoin's ~2140 endpoint).  Era 5 runs forever: miners receive a fraction of
//! the recycle fund each epoch (RECYCLE_REWARD_RATE / RECYCLE_REWARD_DENOM).
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

/// System account that accumulates recycled tokens.
/// Funded by: rounding remainders, blob fees, inference fees, service fees.
/// Drained by: era-5 block rewards (recycle_reward_at).
pub const RECYCLE_FUND_ACCOUNT: &str = "__recycle_fund__";

/// Total clock reward per epoch (paid to ALL clock nodes combined, then split equally).
/// 0.001 BTCPC = 0.05% of the 2 BTCPC block reward.  Tiny enough to not dominate
/// earnings; large enough to incentivize running clock nodes.
pub const CLOCK_REWARD_DREAMS: u64 = 10_000_000;

/// Inference fee split in basis points (10_000 = 100%).
/// Worker receives the bulk; verifiers/reviewers share their pools equally.
/// On the happy path (no dispute): worker 80%, verifiers 15%, recycle 5%.
pub const INFERENCE_FEE_WORKER_BPS: u64 = 8_000;   // 80%
pub const INFERENCE_FEE_VERIFIER_BPS: u64 = 1_500; // 15% split among verifiers
pub const INFERENCE_FEE_RECYCLE_BPS: u64 = 500;    // 5% to recycle fund

/// On the disputed+reviewed path: worker 70%, verifiers 10%, reviewers 15%, recycle 5%.
pub const INFERENCE_FEE_WORKER_DISPUTED_BPS: u64 = 7_000;
pub const INFERENCE_FEE_VERIFIER_DISPUTED_BPS: u64 = 1_000;
pub const INFERENCE_FEE_REVIEWER_BPS: u64 = 1_500; // 15% split among human reviewers
pub const INFERENCE_FEE_RECYCLE_DISPUTED_BPS: u64 = 500;

/// Epochs a worker has to file an InferenceJobClaim after a dispute verdict.
/// At era-0 (30s epochs), 20 epochs ≈ 10 minutes.
pub const CLAIM_WINDOW_EPOCHS: u64 = 20;

/// Minimum verifier votes required to resolve a dispute.
pub const MIN_REVIEW_VOTES: u64 = 3;

// ── Epoch reward pool allocation (basis points, 10_000 = 100%) ──────────────

/// Inference work (output tokens × hw_tier × model_weight).
pub const EPOCH_POOL_INFERENCE_BPS: u64 = 5_000; // 50%
/// Storage nodes (bytes proven in StorageHeartbeat).
pub const EPOCH_POOL_STORAGE_BPS: u64 = 2_000;   // 20%
/// Sensor nodes (reading_count in SensorDataCommit).
pub const EPOCH_POOL_SENSOR_BPS: u64 = 1_500;    // 15%
/// Inference verifiers (jobs verified in InferenceJobVerify).
pub const EPOCH_POOL_VERIFY_BPS: u64 = 1_000;    // 10%
/// Remainder flows to recycle fund (covers clock reward too).
pub const EPOCH_POOL_RECYCLE_BPS: u64 = 500;     // 5%

/// Returns the point multiplier for a hardware tier.
///
/// Higher tiers run larger models and contribute more valuable work.
/// 0=phone, 1=cpu-only, 2=gpu-consumer, 3=gpu-prosumer, 4=gpu-server
#[inline]
pub fn hw_tier_weight(tier: u8) -> u64 {
    match tier {
        0 => 1,
        1 => 4,
        2 => 8,
        3 => 16,
        4 => 32,
        _ => 1,
    }
}

/// Returns the point multiplier for a model name string.
///
/// Based on approximate parameter count extracted from the model identifier.
pub fn model_weight(model: &str) -> u64 {
    let m = model.to_ascii_lowercase();
    if m.contains("0.5b") { return 1; }
    if m.contains("1.5b") || m.contains("1b") { return 2; }
    if m.contains("3b") || m.contains("3.8b") || m.contains("4b") { return 3; }
    if m.contains("7b") || m.contains("8b") { return 6; }
    if m.contains("13b") || m.contains("14b") { return 10; }
    if m.contains("30b") || m.contains("32b") || m.contains("33b")
        || m.contains("mixtral") { return 20; }
    if m.contains("70b") || m.contains("72b") { return 40; }
    2 // unknown model — default to small
}

/// Compute inference contribution score for a single miner's Mine entry.
///
/// score = output_tokens × hw_tier_weight × model_weight
#[inline]
pub fn inference_score(output_tokens: u64, hw_tier: u8, model: &str) -> u64 {
    output_tokens.saturating_mul(hw_tier_weight(hw_tier)).saturating_mul(model_weight(model))
}

/// Per-epoch distribution rate from the recycle fund.
///
/// Each era-5 epoch, the miner receives this fraction of the current fund balance.
///
/// P = 0.00001 → 0.001%/epoch, 0.09%/day (at 90 epochs/day in era 5).
///
/// Buffer time at equilibrium (fund lasts without fees): 1/(P × 90) ≈ 1,111 days.
///
/// Derivation:
///   Starting fund:  840,000 BTCPC (124 years × 0.04 BTCPC/epoch rounding)
///   Target buffer:  ~3 years of coverage against zero fee income
///   Equilibrium:    fund stabilises at fees_per_day / (P × 90)
///   Era-5 has 90 epochs/day; each epoch accumulates 32× more fee transactions
///   than an era-0 epoch (automatic slow-epoch scaling).
pub const RECYCLE_REWARD_RATE: u128 = 10; // numerator
pub const RECYCLE_REWARD_DENOM: u128 = 1_000_000; // denominator → 0.00001 = 10/1_000_000

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

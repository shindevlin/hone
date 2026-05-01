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

/// Base clock reward per epoch at era 0 (30-second epochs).
/// Doubles each era so clock nodes earn the same per-day income regardless of how
/// long epochs get.  Use clock_reward_at(epoch) rather than this constant directly.
pub const CLOCK_REWARD_DREAMS: u64 = 10_000_000; // 0.001 BTCPC at era 0

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

// ── Testnet incentive fund ────────────────────────────────────────────────────

/// System account holding mainnet tokens reserved for testnet operators.
/// Seeded in genesis so nodes running btcpc-satoshi get small mainnet rewards.
pub const TESTNET_FUND_ACCOUNT: &str = "__testnet_fund__";

/// Per registered testnet operator per mainnet epoch (base, era 0).
/// Scales with epoch duration the same way clock_reward_at does.
pub const TESTNET_REWARD_BASE_DREAMS: u64 = 5_000_000; // 0.0005 BTCPC at era 0


/// Per-epoch clock reward that maintains constant daily income as epoch duration grows.
/// Doubles each era: era 0 = 0.001 BTCPC, era 1 = 0.002 BTCPC, era 2 = 0.004 BTCPC …
#[inline]
pub fn clock_reward_at(epoch: u64) -> u64 {
    let shift = era(epoch).min(RECYCLE_ERA);
    CLOCK_REWARD_DREAMS << shift
}

/// Per-operator testnet reward at the given mainnet epoch (scales with clock_reward_at).
#[inline]
pub fn testnet_reward_at(epoch: u64) -> u64 {
    let shift = era(epoch).min(RECYCLE_ERA);
    TESTNET_REWARD_BASE_DREAMS << shift
}

/// Integer square root (Newton's method) — deterministic across all platforms.
pub fn isqrt(n: u64) -> u64 {
    if n == 0 { return 0; }
    let mut x = n;
    let mut y = x.saturating_add(1) / 2;
    while y < x {
        x = y;
        y = x.saturating_add(n / x) / 2;
    }
    x
}

/// Stake-weight multiplier for inference mining: min(isqrt(stake/MIN_STAKE), 10).
/// Bootstrap floor of 1 so unstaked nodes can still participate at minimum rate.
pub fn stake_weight(stake: u64) -> u64 {
    if stake < MIN_STAKE { return 1; }
    isqrt(stake / MIN_STAKE).min(10).max(1)
}

/// Sensor contribution score: type-aware, all-integer, no hard reading cap.
///
/// Different sensor types have fundamentally different value models:
/// - "continuous": complete time-series matters (temp, humidity, power).
///   Value scales with sqrt(readings) — diminishing returns prevent spam domination.
/// - "event": each reading is individually valuable (GPS commit, seismic trigger).
///   Linear up to a hard cap of 20 — duplicates don't add value.
/// - "sampled": periodic snapshots with diminishing returns (air quality, CO2).
///   Linear up to 60 readings, then reduced rate for higher counts.
/// - "pulse": presence/uptime proof. High value for first reading, small for extras.
///
/// Unknown type falls back to conservative sqrt-based scoring.
pub fn sensor_score(reading_count: u64, sensor_type: &str) -> u64 {
    match sensor_type {
        "continuous" => isqrt(reading_count.saturating_mul(100)),
        "event"      => reading_count.min(20).saturating_mul(100),
        "sampled"    => {
            let base  = reading_count.min(60).saturating_mul(30);
            let extra = reading_count.saturating_sub(60).min(940).saturating_mul(5);
            base + extra
        }
        "pulse" => {
            if reading_count == 0 { return 0; }
            200_u64.saturating_add(reading_count.saturating_sub(1).min(10).saturating_mul(10))
        }
        _ => isqrt(reading_count.saturating_mul(10)),
    }
}

/// Mempool relay score: high throughput at low latency earns more.
/// score = entries_relayed * 1000 / max(latency_ms, 1)
pub fn mempool_relay_score(entries_relayed: u64, latency_ms: u64) -> u64 {
    entries_relayed.saturating_mul(1_000) / latency_ms.max(1)
}

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

// ── Activity-gating ───────────────────────────────────────────────────────────

/// Fixed-point denominator for activity ratios (10_000 = 1.0 = 100%).
pub const ACTIVITY_RATIO_DENOM: u64 = 10_000;

/// Minimum activity ratio (in ACTIVITY_RATIO_DENOM units) when some work exists.
/// 100/10_000 = 1%.  An epoch with any participants earns at least 1% of the pool.
/// Epochs with zero participants across all pools earn nothing (ratio = 0).
pub const MIN_ACTIVITY_RATIO_NUM: u64 = 100;

/// Calibration targets — translate each pool's raw units to a dimensionless
/// [0, 1.0] utilization score.  All pools can then be compared fairly when
/// computing activity ratio and pool budget allocation.
///
/// Values represent "a healthy small early-network epoch."  Auto-adjustment
/// (EIP-1559-style, ADJUST_RATE ≈ 0.05%/epoch) and governance can tune them.

/// Inference calibration target — weighted score points per epoch.
/// ≈ 100 inference jobs × 100 output_tokens × hw_tier_weight=1 × model_weight=1
pub const CALIBRATION_INFERENCE: u64 = 10_000;

/// Storage calibration target — (bytes_proven + query_bonus) per epoch.
/// ≈ 1 node × 10 GB stored with moderate query traffic.
pub const CALIBRATION_STORAGE: u64 = 10_000_000_000; // 10 GB

/// Sensor calibration target — sensor_score() total per epoch.
/// ≈ 5 continuous sensors × 1000 readings → sensor_score ≈ 316 each → ~1580 total.
/// ≈ 5 event sensors × 10 readings → sensor_score = 1000 each → ~5000 total.
/// Target set at a mixed healthy network.
pub const CALIBRATION_SENSOR: u64 = 5_000;

/// Verifier calibration target — value_score_total from approved verdicts per epoch.
/// ≈ 50 verifications × avg value_score of 100 per job.
pub const CALIBRATION_VERIFIER: u64 = 5_000;

/// Service calibration target — container_hours per epoch.
/// ≈ 1 service node running 24 hours of containers per epoch.
pub const CALIBRATION_SERVICE: u64 = 24;

/// Mempool calibration target — relay score per epoch.
/// score = entries_relayed * 1000 / max(latency_ms, 1)
/// ≈ 1 fast relay handling 1000 entries at 100 ms latency = 10_000.
pub const CALIBRATION_MEMPOOL: u64 = 10_000;

// ── Scarcity critical mass ────────────────────────────────────────────────────

/// Minimum participant count for a pool to pay out at full rate.
/// Below critical mass: payout_factor = participants / critical_mass (< 1.0).
/// Remainder flows to recycle — prevents early sparse networks from over-extracting.
/// These are STATIC initial values; replace with EMA-based dynamic targets later.
pub const CRITICAL_MASS_INFERENCE: u64 = 10;
pub const CRITICAL_MASS_STORAGE:   u64 = 5;
pub const CRITICAL_MASS_SENSOR:    u64 = 20;
pub const CRITICAL_MASS_VERIFIER:  u64 = 5;
pub const CRITICAL_MASS_SERVICE:   u64 = 3;
pub const CRITICAL_MASS_MEMPOOL:   u64 = 3;
pub const CRITICAL_MASS_TRACKER:   u64 = 5;

// ── BLE tracker calibration ───────────────────────────────────────────────────

/// Tracker coverage calibration target — sensor_score("event") total per epoch.
/// ≈ 5 observers × 4 sighting batches × 100 score = 2_000.
pub const CALIBRATION_TRACKER: u64 = 2_000;

// ── Stake-weighted mining ─────────────────────────────────────────────────────

/// Minimum stake for a non-trivial stake multiplier (100 BTCPC).
/// stake_weight = min(isqrt(stake / MIN_STAKE), 10); bootstrap floor = 1.
pub const MIN_STAKE: u64 = 100 * 10_000_000_000; // 100 BTCPC in dreams

// ── Device claim overbid ──────────────────────────────────────────────────────

/// Minimum overbid multiplier to forcibly claim a device from an existing holder.
/// New stake must be ≥ old_stake × (OVERBID_NUM / OVERBID_DENOM) = 1.5×.
///
/// The 50% increment acts as proof of economic commitment and deters trivial
/// squatting while remaining accessible for legitimate used-device buyers.
/// Old owner receives their full stake back automatically on overbid.
pub const DEVICE_CLAIM_OVERBID_NUM: u128 = 3;
pub const DEVICE_CLAIM_OVERBID_DENOM: u128 = 2;

/// Share of the overbid premium that goes to device yield stakers (in basis points).
/// 5_000 bps = 50%.  Remaining 50% → recycle fund.
/// Old claim owner always receives their full principal back regardless.
/// If no yield stakers exist, the full premium goes to recycle.
pub const OVERCLAIM_STAKER_SHARE_BPS: u64 = 5_000;

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
    fn isqrt_correct() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(9), 3);
        assert_eq!(isqrt(100), 10);
        assert_eq!(isqrt(99), 9);
        assert_eq!(isqrt(u64::MAX), 4294967295);
    }

    #[test]
    fn sensor_score_types() {
        // continuous: sqrt(n * 100); 1440 readings → isqrt(144000) = 379
        assert_eq!(sensor_score(0, "continuous"), 0);
        assert_eq!(sensor_score(1, "continuous"), isqrt(100));  // = 10
        assert_eq!(sensor_score(100, "continuous"), isqrt(10_000)); // = 100
        assert_eq!(sensor_score(1440, "continuous"), isqrt(144_000)); // ≈ 379

        // event: linear, hard cap at 20
        assert_eq!(sensor_score(1, "event"), 100);
        assert_eq!(sensor_score(20, "event"), 2_000);
        assert_eq!(sensor_score(100, "event"), 2_000); // capped

        // sampled: base up to 60, then reduced
        assert_eq!(sensor_score(30, "sampled"), 900);
        assert_eq!(sensor_score(60, "sampled"), 1_800);
        assert_eq!(sensor_score(160, "sampled"), 1_800 + 500); // 900 extra at 5/reading

        // pulse: 200 for first, 10 for extras
        assert_eq!(sensor_score(0, "pulse"), 0);
        assert_eq!(sensor_score(1, "pulse"), 200);
        assert_eq!(sensor_score(5, "pulse"), 240);
        assert_eq!(sensor_score(12, "pulse"), 300); // capped at 10 extras (200 + 10*10)
    }

    #[test]
    fn stake_weight_values() {
        assert_eq!(stake_weight(0), 1);                      // bootstrap
        assert_eq!(stake_weight(MIN_STAKE - 1), 1);          // below min
        assert_eq!(stake_weight(MIN_STAKE), 1);              // sqrt(1) = 1
        assert_eq!(stake_weight(MIN_STAKE * 4), 2);          // sqrt(4) = 2
        assert_eq!(stake_weight(MIN_STAKE * 100), 10);       // sqrt(100) = 10, capped
        assert_eq!(stake_weight(MIN_STAKE * 10_000), 10);    // sqrt(10000) = 100 → capped to 10
    }

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

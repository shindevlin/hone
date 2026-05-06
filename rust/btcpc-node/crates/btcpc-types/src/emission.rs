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
/// Era 1+ durations are NOT derived from this by doubling — they are recomputed at each
/// era boundary by `calibrate_era_epoch_ms` so that supply exhaustion tracks Bitcoin's
/// projected last-coin timestamp.  This constant is only authoritative for era 0.
pub const INITIAL_EPOCH_MS: u64 = 30_000;

// ── Bitcoin alignment — era calibration and oracle layer ─────────────────────
//
// BTCPC supply exhaustion is designed to end within minutes of Bitcoin's last
// mined coin.  No governance action is ever required.  The mechanism has two
// interlocking layers:
//
// LAYER 1 — Compile-time projection (active from genesis)
//   BTC_PROJECTED_END_MS is derived from Bitcoin's genesis timestamp plus
//   6,930,000 blocks × 10-minute average.  At each BTCPC era boundary the
//   chain recomputes the next era's epoch duration so that the remaining supply
//   epochs exhaust exactly at that timestamp.  This self-corrects at every
//   era: eras 1-4 each recalibrate from the current wall-clock time, absorbing
//   any drift that accumulated in the prior era.
//
// LAYER 2 — Oracle refinement (precision grows each decade)
//   Any BTCPC node that also runs a Bitcoin full node may submit a
//   `BtcHeightReport` entry containing the latest observed Bitcoin block height
//   and the average block interval over the most recent 2016-block difficulty
//   window.  The chain holds a 7-entry rolling median of recent reports and
//   stores the derived projected end timestamp in chain_param:btc_end_ms.
//   The era calibration reads this value in preference to BTC_PROJECTED_END_MS.
//
//   Submission rules (enforced in tx.rs):
//     - Signed by an active clock-node posting key (already a trusted role).
//     - Height must be monotonically increasing (no rewinding).
//     - Implied block rate must be in [540_000, 660_000] ms (±10% of 10 min).
//     - Reports older than 720 epochs (~6 hours at era 0) are ignored.
//
//   The oracle is purely additive — existing nodes that do not run a Bitcoin
//   node continue to function normally and benefit from reports submitted by
//   those that do.  No coordination, no voting, no governance.
//
// PRECISION TIMELINE
//   Era 0 (2026-2030): layer 1 only.  Calibration uses pure math; the era 1
//     epoch duration will be computed from the remaining time to BTC_PROJECTED_END_MS.
//   Era 1-3 (2030-2076): oracle reports narrow the deviation as Bitcoin's
//     actual block-time average becomes measurable over multi-decade windows.
//   Era 4 (2076-2140): final new-supply era.  The calibration at the era 4
//     boundary fires when Bitcoin has had 8 additional halvings and its
//     schedule is known to within weeks.  Era 4 epoch durations (~8 minutes)
//     allow end-time precision of ±1 epoch duration.

/// Bitcoin genesis block (block 0) Unix timestamp in milliseconds.
/// 2009-01-03 18:15:05 UTC.
pub const BTC_GENESIS_MS: u64 = 1_231_006_505_000;

/// Bitcoin block height at which the subsidy first rounds to 0 satoshis
/// (after halving 33 at block 6,930,000).
pub const BTC_LAST_BLOCK: u64 = 6_930_000;

/// Average Bitcoin block interval in milliseconds (600 s = 10 minutes).
pub const BTC_AVG_BLOCK_MS: u64 = 600_000;

/// Compile-time projected Unix timestamp (ms) for Bitcoin's last mined coin.
/// = BTC_GENESIS_MS + BTC_LAST_BLOCK × BTC_AVG_BLOCK_MS
/// ≈ 2140-10-01.  Used as the target for era calibration.
/// Nodes may supply a refined estimate via chain_param:btc_end_ms (updated
/// automatically by the oracle path — no governance required).
pub const BTC_PROJECTED_END_MS: u64 =
    BTC_GENESIS_MS + BTC_LAST_BLOCK * BTC_AVG_BLOCK_MS;
// = 1_231_006_505_000 + 6_930_000 × 600_000 = 5_389_006_505_000
// ≈ Unix 5_389_006_505 s ≈ 2140-10-01 18:15 UTC

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

// ── Phase 3: Useful-work proof linkage ───────────────────────────────────────

/// 90-day grace period for unlinked Mine entries (D8).
/// After this epoch count, unlinked Mine earns 0% of inference pool.
/// At era-0 (30s epochs): 90 × 24 × 60 × 2 = 259_200 epochs.
pub const MINE_GRACE_PERIOD_EPOCHS: u64 = 259_200;

/// Reward fraction (in BPS) for unlinked Mine entries during the grace period.
/// 2_000 / 10_000 = 20% of normal inference pool share.
pub const MINE_GRACE_REWARD_BPS: u64 = 2_000;

/// Number of protocol benchmark jobs emitted per epoch from `__testnet_fund__`
/// during the grace period (D8, D16 — open model selection).
pub const MINE_GRACE_BENCHMARK_JOBS_PER_EPOCH: u64 = 5;

/// Verifier random assignment (D9).
/// When enabled, a verifier is only eligible for a job if
/// `sha256(epoch_entropy || job_id || verifier)[0] < VERIFIER_THRESHOLD_BYTE`.
/// 51/256 ≈ 20% chance per verifier per job.
/// Disabled by default until the network has enough active verifiers.
pub const VERIFIER_ASSIGNMENT_ENABLED: bool = false;
pub const VERIFIER_THRESHOLD_BYTE: u8 = 51; // ~20% of 256

/// Commit-reveal for inference verifiers (T4-2).
/// When enabled, verifiers must submit InferenceJobCommit before InferenceJobVerify.
/// The commit hides the verdict until all board members have committed, preventing
/// verdict copying and rubber-stamp collusion.
/// Disabled by default until the network has enough active verifiers; enable via governance.
pub const INFERENCE_COMMIT_REVEAL_ENABLED: bool = false;

/// Epochs after the last commit before reveals are accepted (grace window for slow verifiers).
/// After this window, the commit phase is considered closed.
pub const COMMIT_REVEAL_WINDOW_EPOCHS: u64 = 3;

/// Rolling window (in epochs) for verifier approval-rate tracking (leaky bucket).
/// Verifiers whose approval rate over this window exceeds the rubber-stamp
/// threshold have their weight halved; above the suspend threshold they are
/// suspended for VERIFIER_SUSPEND_EPOCHS.
pub const VERIFIER_APPROVAL_WINDOW_EPOCHS: u64 = 100;

/// Network size thresholds for auto-scaling the verifier board per AI task.
/// Below THRESH_3: 1 verifier (bootstrap). Above THRESH_5: 5 verifiers (mature network).
pub const VERIFIER_ACTIVE_THRESH_3: u64 = 5;
pub const VERIFIER_ACTIVE_THRESH_5: u64 = 15;

/// Rolling window for counting unique active verifiers (used for auto-scaling).
pub const VERIFIER_ACTIVITY_TRACK_EPOCHS: u64 = 10;

/// Voting weights for verifier board quorum resolution.
/// review_required carries +1 so it wins all three-way ties (1/1/1 → 101 > 100).
pub const VERIFIER_VOTE_WEIGHT_NORMAL: u64 = 100; // approved / rejected
pub const VERIFIER_VOTE_WEIGHT_REVIEW: u64 = 101; // review_required — tiebreak winner

/// Approval rate (in BPS) above which a verifier's weight is halved (>95%).
pub const VERIFIER_RUBBER_STAMP_BPS: u64 = 9_500;

/// Approval rate (in BPS) above which a verifier is suspended (>99%).
pub const VERIFIER_SUSPEND_BPS: u64 = 9_900;

/// Number of epochs a rubber-stamp-suspended verifier cannot submit verdicts.
pub const VERIFIER_SUSPEND_EPOCHS: u64 = 20;

/// Inference proof prune window (D1): epochs after InferenceJobPay before
/// payload fields (compute_proof, input_hash, result_hash) are pruneable.
/// Reward and verdict records are kept forever.
pub const INFERENCE_PRUNE_WINDOW_EPOCHS: u64 = 100;

// ── Phase 5: Fee Market ───────────────────────────────────────────────────────

/// Processing weight class for fee calculation (T3-1, T3-2).
/// Fee per entry = base_fee_dreams × entry_weight.
/// System entries (ENTRY_WEIGHT_SYSTEM = 0) are always free.
pub const ENTRY_WEIGHT_SYSTEM:       u64 = 0;
pub const ENTRY_WEIGHT_MICRO:        u64 = 1;  // Transfer, LivenessProof
pub const ENTRY_WEIGHT_STANDARD:     u64 = 3;  // Mine, SensorDataCommit, GatewayHeartbeat
pub const ENTRY_WEIGHT_HEAVY:        u64 = 5;  // InferenceJobPost, StorageHeartbeat
pub const ENTRY_WEIGHT_BULK:         u64 = 10; // BlobStore, LinkGitRefUpdate
pub const ENTRY_WEIGHT_REGISTRATION: u64 = 20; // AccountCreate, ClockNodeRegister

/// Starting base fee per weight unit in dreams (0.001 BTCPC at era-0).
/// Adjusts ±10% per epoch toward EPOCH_TARGET_WEIGHT_UNITS (EIP-1559-style).
pub const BASE_FEE_INITIAL_DREAMS: u64 = 10_000_000; // 0.001 BTCPC per weight unit

/// Target total entry weight per epoch (50% of comfortable epoch capacity).
/// If actual_weight < target → fee falls; if actual_weight > target → fee rises.
pub const EPOCH_TARGET_WEIGHT_UNITS: u64 = 1_000;

/// Maximum base fee change per epoch in BPS: 10% cap (mirrors EIP-1559 12.5%).
pub const BASE_FEE_MAX_CHANGE_BPS: u64 = 1_000;

/// Absolute minimum base fee (0.0000001 BTCPC) — prevents fee dropping to zero.
pub const BASE_FEE_MIN_DREAMS: u64 = 1_000;

/// Absolute maximum base fee (100 BTCPC) — hard cap against runaway increases.
pub const BASE_FEE_MAX_DREAMS: u64 = 100 * 10_000_000_000;

/// Compute the base fee for the next epoch given epoch weight vs. target.
///
/// Formula: `new = old × (1 ± 0.1 × min(|ratio|, 1))` where `ratio = (actual-target)/target`.
/// Integer-safe: uses BPS arithmetic throughout, no floats.
pub fn compute_next_base_fee(current: u64, actual_weight: u64, target_weight: u64) -> u64 {
    if target_weight == 0 { return current; }
    let (over, under) = if actual_weight >= target_weight {
        (actual_weight - target_weight, 0u64)
    } else {
        (0u64, target_weight - actual_weight)
    };
    // ratio_bps = |actual-target|/target × 10_000, clamped to 10_000 (= 100%)
    let ratio_bps = if over > 0 {
        (over as u128 * 10_000 / target_weight as u128).min(10_000) as u64
    } else {
        (under as u128 * 10_000 / target_weight as u128).min(10_000) as u64
    };
    // change = current × BASE_FEE_MAX_CHANGE_BPS/10_000 × ratio_bps/10_000
    //        = current × ratio_bps / 100_000   (10% max × ratio)
    let change = (current as u128 * ratio_bps as u128 / 100_000) as u64;
    let new_fee = if actual_weight >= target_weight {
        current.saturating_add(change)
    } else {
        current.saturating_sub(change)
    };
    new_fee.max(BASE_FEE_MIN_DREAMS).min(BASE_FEE_MAX_DREAMS)
}

// ── Phase 4: Storage and Sensor Proofs ────────────────────────────────���─────

/// Reward fraction (in BPS) for StorageHeartbeat without a valid Merkle range proof (T4-1, D10).
/// 2_000 / 10_000 = 20% of normal storage reward. Full reward requires challenge_response.
pub const STORAGE_PROOF_NO_PROOF_BPS: u64 = 2_000;

/// Chunk size in bytes for storage Merkle challenges (T4-1).
/// Provers must produce a leaf hash + sibling path for a 1 MB chunk at the challenged index.
pub const STORAGE_CHALLENGE_CHUNK_BYTES: u64 = 1_048_576;

/// Score multiplier (in BPS) applied to sensor commits with a verified location proof (T4-3).
/// 13_000 / 10_000 = 1.3× boost. Soft until SENSOR_LOCATION_REQUIRED_EPOCH.
pub const SENSOR_LOCATION_BOOST_BPS: u64 = 13_000;

/// First epoch at which a sensor commit lacking a location registration is rejected (T4-3).
/// 0 = disabled (soft boost only). Governance sets this before mainnet month 3.
pub const SENSOR_LOCATION_REQUIRED_EPOCH: u64 = 0;

/// Maximum total BLE sightings (all tag types summed) a single observer may report per epoch
/// (T4-3). Overridable at runtime via chain_param:max_sightings_per_observer.
pub const MAX_SIGHTINGS_PER_OBSERVER_PER_EPOCH: u64 = 500;

/// ── Coverage reporting (cellular dead-spot mapping) ──────────────────────────

/// Grid resolution for coverage corroboration (degrees × 1000 → integer grid cell).
/// 0.001° ≈ 111m at equator. Reports within the same grid cell contribute to corroboration.
pub const COVERAGE_GRID_RESOLUTION: i64 = 1000;

/// Base score for a signal-present coverage report (dBm known).
pub const COVERAGE_SCORE_SIGNAL: u64 = 100;

/// Base score for a dead-spot report (signal_dbm = None or < COVERAGE_DEAD_SPOT_DBM_THRESHOLD).
/// Higher than signal-present because dead-spot data is rarer and more commercially valuable.
pub const COVERAGE_SCORE_DEAD_SPOT: u64 = 300;

/// Minimum signal strength (dBm) to be classified as "present". Below this → dead spot.
pub const COVERAGE_DEAD_SPOT_DBM_THRESHOLD: i32 = -120;

/// Minimum distinct reporters in the same grid cell + carrier + epoch for corroboration bonus.
pub const COVERAGE_CORROBORATION_MIN_REPORTERS: u64 = 3;

/// Bonus multiplier (BPS) applied to all reporters in a corroborated cell.
/// 15_000 / 10_000 = 1.5× bonus for independently confirmed readings.
pub const COVERAGE_CORROBORATION_BONUS_BPS: u64 = 15_000;

/// Maximum coverage reports a single account may submit per epoch (anti-spam).
pub const COVERAGE_MAX_REPORTS_PER_EPOCH: u64 = 200;

/// Maximum distinct reporters whose votes count toward the corroboration threshold per cell.
/// Reporters beyond this cap still earn base score but cannot trigger the corroboration bonus.
/// Prevents a Sybil farm from inflating the bonus across a large controlled grid cell.
pub const COVERAGE_MAX_CORROBORATING_REPORTERS: u64 = 10;

/// Maximum plausible movement speed for a GNSS sensor between consecutive epochs (m/s).
/// 300 m/s ≈ 1080 km/h — generous upper bound covering aircraft; clearly implausible above this.
/// Applied to the Haversine distance between consecutive lat/lon readings (T4-3).
pub const SENSOR_GNSS_MAX_SPEED_M_S: f64 = 300.0;

/// Epoch duration in seconds, used for GNSS plausibility velocity calculation.
/// Must stay in sync with EPOCH_MS / 1000.
pub const EPOCH_DURATION_S: f64 = 30.0;

/// Liveness rewards: disabled until website page and whitepaper section ship (D2).
pub const LIVENESS_REWARDS_ENABLED: bool = false;

/// Chain entropy decay: disabled until docs ship (D2, T6-4).
pub const ENTROPY_DECAY_ENABLED: bool = false;

// ── T3-4: EMA-based dynamic critical mass ────────────────────────────────────

/// Rolling window for critical-mass EMA estimates.
/// 7 days × 2880 epochs/day (era-0, 30 s) = 20_160 epochs.
pub const EMA_WINDOW_EPOCHS: u64 = 20_160;

/// EMA smoothing numerator: alpha = EMA_ALPHA_NUM / EMA_ALPHA_DENOM = 2 / 20161 ≈ 0.0001.
pub const EMA_ALPHA_NUM: u64 = 2;
/// EMA smoothing denominator: EMA_WINDOW_EPOCHS + 1.
pub const EMA_ALPHA_DENOM: u64 = 20_161;

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

/// Compute the base epoch duration (ms) for `new_era` so that all remaining
/// supply epochs exhaust at `target_end_ms`, given the current wall-clock
/// time `now_ms`.
///
/// Called automatically in the epoch-seal handler whenever the era number
/// increments.  Stores the result per era so `epoch_duration_ms_dynamic`
/// can read it.  No governance action ever required.
///
/// The formula: remaining eras have durations D, 2D, 4D, … (geometric),
/// so total remaining time = DOUBLING_INTERVAL × D × (2^remaining_eras − 1).
/// Solving for D gives the return value.
pub fn calibrate_era_epoch_ms(new_era: u64, now_ms: u64, target_end_ms: u64) -> u64 {
    if new_era >= RECYCLE_ERA {
        // Recycle era runs forever — freeze at the era-5 ceiling.
        return INITIAL_EPOCH_MS << RECYCLE_ERA;
    }
    let remaining_ms = target_end_ms.saturating_sub(now_ms);
    let remaining_eras = RECYCLE_ERA - new_era; // ≥ 1
    // Geometric sum for remaining_eras terms starting at 1: 2^n − 1
    let series_sum = (1u64 << remaining_eras).saturating_sub(1).max(1);
    let base = remaining_ms
        / DOUBLING_INTERVAL.saturating_mul(series_sum).max(1);
    base.max(1_000) // never shorter than 1 second
}

/// Epoch duration in milliseconds at a given epoch — pure / era-0 path.
///
/// This is only authoritative for era 0 (30-second epochs).  For era 1+
/// use `Chain::epoch_duration_ms_dynamic`, which reads per-era calibrated
/// values stored in chain state at each era boundary.
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

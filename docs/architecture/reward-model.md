---
title: HONE Reward Model — Canonical 4-Layer Design
description: Authoritative design for emission, reward pools, verification, sensors, and storage
captured_at: 2026-04-29
author: Shin Devlin
---

# HONE Reward Model — Canonical 4-Layer Design

> An epoch can earn LESS than 100% of the available reward. It can NEVER earn more.
> Unused reward always flows to hone_recycle. Tokens are never burned.

## Master Formula

```
adjusted_ceiling  = block_reward × long_term_scalar          // Layer A
total_emission    = base + activity_pools + fee_boost        // Layers D + B + C
total_emission    = min(total_emission, adjusted_ceiling)    // hard cap
recycle          += adjusted_ceiling - total_emission        // never burned
```

## Layer D — Infrastructure Base (always-on nodes)

Only TRUE infrastructure earns base rewards.

```
base = Σ clock_reward_at(epoch)   per active clock node     (era-scaled, tiny)
     + Σ testnet_reward_at(epoch) per registered testnet op  (era-scaled, tinier)
     + 2% mandatory reserve split:
           1.5% → hone_recycle
           0.5% → __testnet_fund__
```

- clock_reward_at(era 0) = 0.001 HONE/node
- Verifiers do NOT get a base reward. They are workers, not infrastructure.

## Layer B — Per-Pool Activity (fully dynamic)

```
activity_budget = adjusted_ceiling - base_emission

utilization_p   = actual_activity_p / calibration_target_p   // dimensionless 0→1
total_u         = Σ utilization_p (active pools only)
pool_emission_p = (utilization_p / total_u) × activity_budget
```

### Node Types

| Node Type       | Pool basis                          | Also earns              |
|-----------------|-------------------------------------|-------------------------|
| Miners          | verified value_score                | job escrow worker_amount|
| Storage         | bytes_proven × query bonus (2×)     | contract access fees    |
| Sensors         | readings, capped at 10/sensor       | SensorDataPurchase      |
| Verifiers       | verifications completed (0 if idle) | job escrow payments     |
| Service nodes   | active container-hours              | service usage fees      |
| Clock nodes     | heartbeats (Layer D + Layer B both) | —                       |

No static percentages. Market decides the split. Calibration targets normalise units.

### Calibration Target Auto-Adjustment
```
new_target = old_target × (1 + ADJUST_RATE × (actual - old_target) / old_target)
```
ADJUST_RATE ≈ 0.05% per epoch max. Very slow drift. Governance can nudge.

## Layer A — Long-Term Scalar (90-day mean reversion)

```
α_fast = 1 - exp(-ln(2) / ~7-day-half-life-in-epochs)
α_slow = 1 - exp(-ln(2) / ~90-day-half-life-in-epochs)

slow_ema = α_slow × utilization + (1 - α_slow) × slow_ema
fast_ema = α_fast × utilization + (1 - α_fast) × fast_ema
deviation = fast_ema - slow_ema

scalar = lerp(0.7, 1.0, slow_ema) + k × deviation
scalar = clamp(scalar, 0.7, 1.0)
```

Ceiling floor: 70%. Ceiling cap: 100%. Any deviation decays with 7-day half-life.

## Layer C — Fee-Driven Boost (previous epoch fees)

```
verified_fee_vol  = Σ net_flows(epoch-1) from approved-verdict jobs, new-capital only
fee_boost_factor  = min(verified_fee_vol × FEE_BOOST_RATE, MAX_BOOST)
```

Three protections against circular payment attacks:
1. Only fees from `approved` verdict jobs count.
2. Net flow accounting per address pair — circular payments cancel to near-zero.
3. Previous epoch lag + new-capital taint — fees from epoch N-1 boost epoch N.

## Verification Architecture

Verifiers receive (prompt + answer) encrypted to their **memo public key**.
They determine: "Is this real work?" — NOT checking correctness, checking REALNESS.

```
value_score = output_tokens × hw_tier_weight × model_weight × complexity_factor
```
complexity_factor: 1=trivial, 2=moderate, 4=complex (verifier-assessed, bounded).

### Verifier Claim Flow
1. Worker completes job → submits result_hash on-chain
2. Verifier submits `InferenceVerifyClaim { job_id, verifier, epoch }`
3. Worker encrypts (prompt + result) to verifier's memo public key
4. Verifier decrypts → reads prompt and answer
5. Verifier submits `InferenceJobVerify { job_id, verifier, verdict, value_score, epoch }`
6. `approved` value_score feeds into clock's epoch mining pool computation

## Sensor Data — Two Income Streams
- **Baseline** (Layer B): readings_count capped at 10/sensor per epoch.
- **Purchase premium**: SensorDataPurchase fee → sensor owner (majority) + storage + recycle.

## Storage — Two Income Streams
- **Baseline** (Layer B): bytes_proven with query activity bonus (up to 2×).
- **Contract fees**: BlobStore access, LinkGit push/pull, SensorDataPurchase rate.

## Anti-Double-Pay Rule
Verifiers earn from BOTH Layer B pool AND job escrow — two independent streams, intentional.
The ONLY removed double-pay was the old verify_pool counting from InferenceJobVerify events
(same events that already paid from escrow). Now verifier Layer B is based on verification
activity count, which is separate from escrow.

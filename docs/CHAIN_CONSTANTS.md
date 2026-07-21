# HONE Chain Constants — Canonical Reference

This file is the single source of truth for all protocol constants.
The CI workflow `check-constants.yml` fails if any constant listed here
drifts from the value in source code.

---

## Identity

| Constant | Value | Source |
|----------|-------|--------|
| Mainnet chain ID | `hone` | `hone-types/src/lib.rs:MAINNET_CHAIN_ID` |
| Testnet chain ID | `hone-testnet` | `hone-types/src/lib.rs:TESTNET_CHAIN_ID` |
| Native token symbol | `HONE` | `hone-types/src/lib.rs:NATIVE_TOKEN` |
| HTTP API port | `4242` | `src/config.rs` default |
| P2P port | `6942` | `src/config.rs` default |

---

## Token Economics

| Constant | Value | Notes | Source |
|----------|-------|-------|--------|
| Total supply cap | 42,000,000 HONE | Hard maximum, never burned | `emission.rs:SUPPLY_CAP_HUNITS` |
| Smallest unit | 1 hunit | | `lib.rs:HUNITS_PER_HONE` |
| Dreams per HONE | 10,000,000,000 | 10^10 | `lib.rs:HUNITS_PER_HONE` |
| Block reward (era 0) | 2 HONE per epoch | Constant within era | `emission.rs:BLOCK_REWARD_HUNITS` |
| Era 0 epoch duration | 30 seconds | | `emission.rs:INITIAL_EPOCH_MS` |
| Doubling interval | 4,200,000 epochs | Epoch duration doubles per era | `emission.rs:DOUBLING_INTERVAL` |
| Recycle-only era | Era 5 | No new supply after this | `emission.rs:RECYCLE_ERA` |
| Supply exhaustion | ~124 years from genesis | ~2150 | derived |
| Genesis timestamp | 1783191600000 ms | 2026-07-04 noon Los Angeles (PDT, UTC-7) | `src/config.rs` default |

---

## Emission Model

| Era | Epoch duration | Epochs/day | Daily emission | Cumulative years |
|-----|---------------|-----------|----------------|-----------------|
| 0 | 30 s | 2,880 | 5,760 HONE | ~4 years |
| 1 | 60 s | 1,440 | 2,880 HONE | ~8 years |
| 2 | 2 min | 720 | 1,440 HONE | ~16 years |
| 3 | 4 min | 360 | 720 HONE | ~32 years |
| 4 | 8 min | 180 | 360 HONE | ~64 years |
| 5+ | 16 min | 90 | recycle only | ∞ |

---

## Reward Pools

| Pool | Calibration target | Critical mass | Source |
|------|--------------------|---------------|--------|
| Inference | 10,000 score points/epoch | 10 miners | `emission.rs` |
| Storage | 10,000,000,000 bytes/epoch | 5 nodes | `emission.rs` |
| Sensor | 5,000 score/epoch | 20 sensors | `emission.rs` |
| Verifier | 5,000 value_score/epoch | 5 verifiers | `emission.rs` |
| Service | 24 container-hours/epoch | 3 nodes | `emission.rs` |
| Mempool | 10,000 relay score/epoch | 3 nodes | `emission.rs` |
| Tracker | 2,000 sighting score/epoch | 5 observers | `emission.rs` |

---

## Clock Rewards

| Constant | Value | Source |
|----------|-------|--------|
| Clock reward base (era 0) | 0.001 HONE / epoch | `emission.rs:CLOCK_REWARD_HUNITS` |
| Clock reward scaling | Doubles each era (constant daily income) | `emission.rs:clock_reward_at()` |
| Active clock nodes | Top 25 by stake | Phase 2 — D6 |
| Standby pool | Nodes 26–100 | Phase 2 — D6 |

---

## Inference Marketplace

| Constant | Value | Source |
|----------|-------|--------|
| Worker share (happy path) | 80% | `emission.rs:INFERENCE_FEE_WORKER_BPS` |
| Verifier share (happy path) | 15% split among verifiers | `emission.rs:INFERENCE_FEE_VERIFIER_BPS` |
| Recycle share (happy path) | 5% | `emission.rs:INFERENCE_FEE_RECYCLE_BPS` |
| Worker share (disputed) | 70% | `emission.rs:INFERENCE_FEE_WORKER_DISPUTED_BPS` |
| Verifier share (disputed) | 10% | `emission.rs:INFERENCE_FEE_VERIFIER_DISPUTED_BPS` |
| Reviewer share (disputed) | 15% | `emission.rs:INFERENCE_FEE_REVIEWER_BPS` |
| Claim window | 20 epochs (~10 min at era 0) | `emission.rs:CLAIM_WINDOW_EPOCHS` |
| Min review votes | 3 | `emission.rs:MIN_REVIEW_VOTES` |
| Proof pruning window | 100 epochs after InferenceJobPay | D1 |

---

## Staking

| Constant | Value | Source |
|----------|-------|--------|
| Min stake for weight multiplier | 100 HONE | `emission.rs:MIN_STAKE` |
| Stake weight formula | `min(sqrt(stake / MIN_STAKE), 10)` | `emission.rs:stake_weight()` |
| Name registration stake | 10 HONE | `lib.rs:NAME_REGISTRATION_STAKE` |
| Device overbid multiplier | 1.5× minimum | `emission.rs:DEVICE_CLAIM_OVERBID_*` |
| Overbid staker share | 50% of premium | `emission.rs:OVERCLAIM_STAKER_SHARE_BPS` |

---

## System Accounts

| Account | Purpose |
|---------|---------|
| `__recycle_fund__` | Accumulates all fees, slashes, rounding — distributed in era 5 |
| `__testnet_fund__` | Testnet operator rewards and benchmark job funding |
| `@legal` | Slash recipient for 10% of double-sign penalties (controlled by josh) |

---

## Liveness Rewards

Disabled until website and whitepaper documentation ships (`LIVENESS_REWARDS_ENABLED = false`).

| Parameter | Value |
|-----------|-------|
| Years 0–3 | No contributions |
| Years 3–5 | Warning and countdown only |
| Year 5+ | Dormant accounts contribute 10%/year |
| Contribution split | 50% to active live wallets, 50% to `__recycle_fund__` |
| "Active" definition | Any account with a liveness proof in the epoch |

---

## Network Bootstrap

| Parameter | Value |
|-----------|-------|
| Bootstrap peers | `bootstrap1.honemesh.net:6942`, `bootstrap2.honemesh.net:6942` |
| Bootstrap master | shindevlin node — seal always accepted during bootstrap |
| BFT activation | Via governance vote when node count warrants |
| Grace period (mining) | 90 days — unlinked Mine = 20% reward. After: 0% |

---

*Last updated: v0.4.1 — Phase 0 hardening*

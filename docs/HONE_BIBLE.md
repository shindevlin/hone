# THE HONE BIBLE

*The canonical reference for the HONE sovereign multi-resource AI network.*

---

## How to Read This Document

This Bible is assembled from section drafts written by researchers who each read the real code. It is the **single canonical reference** for what HONE is, how its economics work, and where the line falls between what runs today and what is designed but not yet live.

Every claim carries one of three status tags:

- **[BUILT]** — implemented and running in the Rust node (`rust/hone-node/`); citation points to the file/line.
- **[SPEC'D]** — designed in a protocol doc, entry schema present, or partially wired; not fully live.
- **[ASPIRATIONAL]** — a stated goal or roadmap item; no working implementation yet.

Citations point at the real source (`main.rs`, `chain.rs`, `emission.rs`, `entry.rs`, `tx.rs`, `docs/…`). Where a doc and the code disagree, the code wins and the discrepancy is noted.

### Naming Lock (HONE / hunit / hone-\*)

Brand/network = **HONE**, token/ticker = **HONE**, base unit = **hunit** (1 HONE = 10^10 hunits), chain_id = **hone** (mainnet) / **hone-testnet**, crates = `hone-*`, CLI binary = `hone`, env vars = `HONE_*`, domain = **honemesh.net**. Any remaining "BTCPC/btcpc/dream" in older docs or directory names is a **not-yet-migrated tail**, not the current name. The active implementation is Rust; the Node.js prototype (`src/`, `bin/`, `package.json`) is deprecated and referenced only where a doc still describes it.

---

## Table of Contents

- **Part I — What HONE Is + The Economic Engine**
  - What HONE Is
  - The 4-Layer Reward Model (Layers A/B/C/D)
  - Recycle, Not Burn
- **Part II — Consensus & Clock**
  - The Epoch Is the Block
  - Seal → Epoch Resolution
  - Reward Finality, Registration, ClockReward
  - Bootstrap Grace, Slashing, Entropy
  - The No-Peers Hardline
- **Part III — The Resource Economy**
  - Ch. 1 — Inference & Mining
  - Ch. 2 — Sensors & IoT (Verasens)
  - Ch. 3 — Storage (HDD) & Data
  - Ch. 4 — Network Services (Mempool / Service / Runtime / Gateway)
- **Part IV — Identity, Keys & Security**
- **Part V — Applications** (Commerce/Freeport, LinkGit, Projects, Contracts, Bridges, Wiiv)
- **Part VI — Devices, Clients & Roadmap**
- **Appendix — Master Reward-Pool Table**

---

# Part I — What HONE Is + The Economic Engine

## What HONE Is

HONE is a **sovereign multi-resource AI network** — a single-binary Rust chain node (`rust/hone-node/src/main.rs`) where phones, GPUs, HDDs, sensors, and relays are all **full nodes** that get **paid for the real resources they contribute**, priced continuously against live network state. It is not an inference network with side-gigs; inference is one pool among many, all settled by the same engine each epoch.

| Property | Value | Source |
|---|---|---|
| Token / base unit | HONE / hunit; 1 HONE = 10^10 hunits | `HUNITS_PER_HONE` [BUILT] `crates/hone-types/src/lib.rs:12` |
| Total new supply | 42,000,000 HONE | `SUPPLY_CAP_HUNITS` [BUILT] `emission.rs:107` |
| Per-epoch block reward (eras 0–4) | 2 HONE (20,000,000,000 hunits) | `BLOCK_REWARD_HUNITS` [BUILT] `emission.rs:103` |
| Epoch = block, era 0 | 30 s | `EPOCH_MS` / `INITIAL_EPOCH_MS` [BUILT] `lib.rs:18`, `emission.rs:36` |
| Chain IDs | `hone` / `hone-testnet` | [BUILT] `lib.rs:21-22` |
| Transport / state / API | libp2p gossipsub (6942) / RocksDB / Axum (4242) | [BUILT] CLAUDE.md, `main.rs` |

**Emission is time-doubling, not reward-halving** [BUILT `emission.rs`]. The per-epoch reward stays 2 HONE; instead epoch *duration* doubles every `DOUBLING_INTERVAL = 4,200,000` epochs (era 0: 30 s → era 4: ~8 min). Daily emission halves per era exactly as Bitcoin's does. New supply exhausts after 5 eras (~124 years, ~2150), self-calibrated to land within minutes of Bitcoin's last coin via a compile-time projection plus an optional additive `BtcHeightReport` oracle — **no governance ever required** (`emission.rs:38-99, 660-718`). Era 5+ pays purely from recycled tokens (`recycle_reward_at`, `RECYCLE_ERA=5`).

**Hardline invariant** [BUILT CLAUDE.md, `api.rs`]: a node with **zero peers** MUST NOT accept or apply any user-submitted entry — no offline/local mode — or it silently forks. Entries flow *gossip → pending pool → epoch seals → validate_and_apply in sha256 order → RocksDB*. Reward (system) entries apply immediately on seal, bypassing the pending pool.

---

## The Economic Engine — 4-Layer Reward Model

This is the spine every other Bible section hangs off. **Everything paid on HONE is paid based on network state**, computed once per epoch in the seal handler `distribute_epoch_rewards` [BUILT `main.rs:1373+`], following the canonical design in [SPEC'D `docs/architecture/reward-model.md`]. The core promise, enforced by a hard cap: **an epoch can earn LESS than 100%, never more. Unused reward recycles — it is never burned.**

### Master formula [BUILT `main.rs:1389-1426, 1846-1889`; SPEC'D reward-model.md]

```
raw_pool          = block_reward_at(epoch)                     // 2 HONE, era 0-4
adjusted_pool     = raw_pool × long_term_scalar                // Layer A (0.70–1.00)
layer_a_damped    = raw_pool − adjusted_pool        → recycle  // deferred, not burned
reserve_total     = 2% of adjusted_pool                        // Layer D reserve split
activity_pool     = adjusted_pool − reserve_total
gated_pool_base   = activity_pool × activity_ratio             // Layer B activity gate
idle_before_c     = activity_pool − gated_pool_base
fee_boost         = min(net_verified_fees(epoch-1) × ½, idle_before_c)  // Layer C
gated_pool        = gated_pool_base + fee_boost
idle_recycle      = idle_before_c − fee_boost       → recycle
pool_emission_p   = gated_pool × (util_p / Σ util)             // Layer B per-pool split
```

The Layer A scalar is applied to `raw_pool` **before** the reserve split, so the entire epoch ceiling — reserve included — scales with long-run health.

### Layer D — Infrastructure base (always-on) [BUILT `main.rs:1418-1536`; SPEC'D reward-model.md §Layer D]

Only true infrastructure earns a base, independent of activity:

| Component | Rate (era 0) | Scaling | Pool / entry |
|---|---|---|---|
| Clock nodes | 0.001 HONE/seal, ×0.5–1.0 by uptime | doubles per era (`clock_reward_at`) | **ClockReward** |
| Testnet operators | 0.0005 HONE/op, from `__testnet_fund__` | `testnet_reward_at` | **TestnetReward** |
| Reserve split (2%) | 1.5% recycle · 0.4% `__testnet_fund__` · 0.1% `__treasury__` | of adjusted_pool | credited direct |

Clock uptime is a 100-epoch sliding window; sealers get `0.5×–1.0×`, new nodes a 10-epoch grace at full rate (`main.rs:1442-1506`). Verifiers get **no** base — they are workers, not infrastructure.

### Layer B — Per-pool activity (fully dynamic, the market decides the split) [BUILT `main.rs:1806-1889`; SPEC'D reward-model.md §Layer B]

There are **no static percentages**. Each resource type is a POOL. Its share of `gated_pool` is proportional to its **utilization** — its raw work normalized by a per-pool calibration target so unlike units (tokens, bytes, container-hours) compare fairly:

```
util_p          = min(raw_work_p / calibration_target_p, 1.0)      // norm(), dimensionless
Σ util          = sum over active pools
pool_emission_p = gated_pool × util_p / Σ util
activity_ratio  = (Σ util / active_pool_count), floored at MIN_ACTIVITY_RATIO (1%)
```

The nine live Layer-B pools, each a distinct reward entry type:

| Pool | Raw-work basis | Calibration target | Reward entry | Also earns |
|---|---|---|---|---|
| Inference/mining | `inference_score` = tokens × hw_tier × model_weight | `CALIBRATION_INFERENCE`=10,000 | **MineReward** | job escrow (worker 80%) |
| Storage | bytes_proven (Merkle-challenged) + query bonus | `CALIBRATION_STORAGE`=10 GB | **StorageReward** | BlobStore / LinkGit access fees |
| Sensors | `sensor_score` (type-aware, continuous/event/sampled/pulse) | `CALIBRATION_SENSOR`=5,000 | **SensorReward** | SensorDataPurchase premium |
| Verifiers | approved-verdict value_score count | `CALIBRATION_VERIFIER`=5,000 | **VerifierReward** | job escrow (15% split) |
| Service | container-hours | `CALIBRATION_SERVICE`=24 | **ServiceReward** | service usage fees |
| Mempool | `mempool_relay_score` = relayed × 1000 / latency | `CALIBRATION_MEMPOOL`=10,000 | **MempoolReward** | tx-fee share |
| Tracker/coverage | BLE `sensor_score("event")` / coverage grid | `CALIBRATION_TRACKER`=2,000 | **TrackerCoverageReward** | — |
| LinkGit serve | unique remote fetch events | `CALIBRATION_LINKGIT`=100 | **LinkGitServeReward** | — |
| LinkGit build | ref-update pushes | `CALIBRATION_LINKGIT_BUILD`=10 | **LinkGitBuildReward** | — |

Additional pool types exist as entry types and are settled through the same machinery: **RuntimeReward** (decentralized runtime, `CALIBRATION_RUNTIME`=3, host 80%/recycle 20%) and **GatewayRewardSplit** (gateway heartbeats) [BUILT `entry.rs:1058,1168,1179`; emitted `main.rs:1949`]. Clock nodes uniquely earn on **both** Layer D and Layer B.

**Calibration targets auto-adjust** slowly (EIP-1559-style, `ADJUST_RATE ≈ 0.05%/epoch`) and can be nudged by governance — they normalize units, they do not set winners [SPEC'D reward-model.md §Calibration]. A separate **scarcity / critical-mass** gate scales sparse pools below their `CRITICAL_MASS_*` participant count (`payout_factor = participants / critical_mass`), sending the remainder to recycle so an early sparse network cannot over-extract [BUILT `emission.rs:386-393`, `main.rs`; SPEC'D core-design.md §Scarcity]. Critical-mass targets are moving toward EMA-based dynamic values (`EMA_WINDOW_EPOCHS=20,160`) [BUILT `main.rs:1540-1545`].

### Layer C — Fee-driven boost (anti-circular) [BUILT `main.rs:1849-1884`; SPEC'D reward-model.md §Layer C]

Genuine paid demand **unlocks idle rewards that would otherwise recycle** — it never mints new supply (capped at `idle_before_c`). Three protections against wash-trading:

1. **Previous-epoch lag** — only `fee_flow:{epoch-1}` entries count (new-capital taint).
2. **Net-flow accounting per address pair** — circular rings A→B→A cancel to ~0 (canonical unordered-pair netting, `main.rs:1866-1873`).
3. Boost = `net_verified_fees × ½` (`LAYER_C_FEE_BOOST_NUM/DENOM`), then `min(…, idle_before_c)`.

Per reward-model.md, only fees from **`approved`-verdict** jobs qualify — the boost cannot be self-funded by unverified work.

### Layer A — Long-term scalar (90-day mean reversion) [BUILT `emission.rs:203-252`, `main.rs:1410-1416`; SPEC'D reward-model.md §Layer A]

A two-speed integer EMA of total pool utilization yields a ceiling scalar in **[0.70, 1.00]** (`LAYER_A_SCALAR_MIN=7000`, `MAX=10000`), applied to `raw_pool` each epoch. It uses `min(fast, slow)` so a recent utilization dip deflates the ceiling fast, while recovery requires both a 7-day (`EMA_ALPHA_DENOM=20,161`) and a 90-day (`LAYER_A_SLOW_ALPHA_DENOM=259,201`) window to climb back together. Damped tokens flow to recycle, deferred until the network fills back up — never destroyed (tests confirm: 30 idle epochs barely move it; ~200k idle epochs pull it to the 0.70 floor).

### Recycle, not burn [BUILT `emission.rs:110-115`, `main.rs:1428-1436,1882-1884`]

Every leftover path — Layer A damping, sub-100% activity gate, unspent Layer C headroom, reserve split, rounding remainders, rejected/timed-out permissive tokens — credits `__recycle_fund__` (`RECYCLE_FUND_ACCOUNT`). In era 5+ that fund pays the only reward (`RECYCLE_REWARD_RATE/DENOM = 10/1,000,000` per epoch), giving the network an infinite-horizon, self-sustaining tail. **Nothing is ever burned.**

> Every subsequent Bible section (mining, storage, sensors, clock, verification, commerce, gateway, runtime) plugs into exactly this engine: it names its **reward entry type** and its **Layer-B pool**, and its pay is `gated_pool × util_p / Σ util` — priced live against everything else the network did that epoch.

---

# Part II — Consensus & Clock

> **The epoch IS the block.** HONE has no separate block producer. Registered clock nodes each broadcast an `EpochSeal` over gossip; the median/quorum of those seals *is* the block for that epoch. Every 30 s the sealed epoch is the commit boundary at which pending user entries are drained, sorted, and applied. The clock is HONE's most primitive contributed resource — its own reward **pool** (`ClockReward`, Layer D infrastructure base), paid every epoch regardless of what other work happened.

### Core parameters

| Parameter | Value | Source |
|---|---|---|
| Epoch (block) duration | 30 s (`EPOCH_MS = 30_000`) | [BUILT] `clock.rs:21` |
| Seal collection window | 5 s (`SEAL_COLLECT_MS`) | [BUILT] `clock.rs:17` |
| Peer-seal extra wait (have peers, 1 seal) | 20 s (`PEER_SEAL_WAIT_MS`) | [BUILT] `clock.rs:19` |
| Reward-consensus collection window | 10 s (`CONSENSUS_COLLECT_MS`) | [BUILT] `clock.rs:24` |
| Quorum fraction | >51% (`MIN_QUORUM_FRACTION = 0.51`) | [BUILT] `clock.rs:23` |
| Min unique sealers | 2 (`HONE_CLOCK_QUORUM` env, default) | [BUILT] `clock.rs:28-33` |
| Outlier timestamp tolerance | 2 epochs = 60 s (`OUTLIER_EPOCH_TOLERANCE`) | [BUILT] `clock.rs:20,472` |
| Isolation → observer mode | 3 epochs with no external peers | [BUILT] `clock.rs:22,318-322` |
| Clock stake minimum | 5 HONE (`chain_param:clock_min_stake`) | [BUILT] `clock.rs:620`, `chain.rs:2972` |
| Bootstrap grace end | epoch 100_000 (`CLOCK_BOOTSTRAP_GRACE_END_EPOCH`) | [BUILT] `hone-types/src/lib.rs:45` |

### Seal → epoch resolution pipeline [BUILT] `clock.rs`

1. **Ingest** — `receive_seal()` parses an `EpochSeal { epoch_number, node_id, timestamp, seal_hash, signature, pubkey }` from gossip. If a `signature` is present it is verified (ed25519) against `pubkey` (or `node_id` if it is a 64-char raw pubkey). Bad-sig seals are dropped; unsigned seals are allowed. Seals are deduplicated per `node_id` and buffered in an `EpochState` with a 5 s deadline. `clock.rs:224-273`
2. **Tick** — `tick()` (called ~1/s from the main loop) resolves any `EpochState` whose deadline passed. No early-resolve on receipt — keeps the collection window consistent across peers. `clock.rs:277-291`
3. **Resolve** — `resolve_epoch()` decides the winner by branch (below), scores every sealer, emits a `SealedEpoch`, and prunes epoch states older than 20 behind. `clock.rs:374-569`

**Resolution branches** (`clock.rs:420-557`):

| Case | Behaviour |
|---|---|
| Bootstrap master | If `registered_clocks` empty **and** 0 external peers (or `HONE_BOOTSTRAP_ISOLATION=true`), `shindevlin`'s seal is the sole quorum-1 winner. `clock.rs:398-432` |
| No seals | Epoch marked resolved but `sealed=false` — skipped, chain does not advance. `clock.rs:433-444` |
| One seal | If peers exist, wait up to `PEER_SEAL_WAIT_MS` (20 s) for a peer seal; else self-seal quorum-1 (isolated single clock). `clock.rs:445-470` |
| Multi-seal (normal) | Median timestamp → drop outliers (>60 s deviation) → keep **registered** inliers → require `ceil(denominator×0.51)` → winner = most common `seal_hash`. `signing_clocks` = all sealers on the winning hash. `clock.rs:471-557` |

The winning-hash set becomes `SealedEpoch.signing_clocks` — **this list is exactly who earns `ClockReward`.** `clock.rs:59`, consumed at `main.rs:474-475`.

### Epoch-seal handler (the commit boundary) [BUILT] `main.rs:326-506`

On each `sealed && sealed==true` event the node, in order:
1. Applies the `EpochSeal` ledger entry. `main.rs:338-348`
2. **`drain_pending_sorted()`** → applies pending user entries via `tx::validate_and_apply` in sha256 order (identical gossip set ⇒ identical order across nodes). This is where entries become "on-chain." `main.rs:354-380`
3. Hardware-conflict check (one HONE account per physical machine; exits on loss). `main.rs:382-397`
4. Drains unbonding, timelocked params, governance; recomputes EIP-1559-style base fee. `main.rs:405-430`
5. Refreshes registered clock set, persists `epoch_validators:{epoch}` snapshot, sets it as the live quorum denominator. `main.rs:450-458`
6. Computes and stores epoch entropy. `main.rs:460-469`
7. **`emit_epoch_rewards(...)`** — the full 4-layer multi-resource distribution across all pools. `main.rs:474-475`
8. Writes the sealed block, computes `rewards_hash`, broadcasts a `RewardProposal` on `hone/consensus`. `main.rs:477-497`

### Reward finality (two-phase) [BUILT] `clock.rs:165-220`, [SPEC'D] `docs/CONSENSUS.md`

Sealing advances chain state; **finality** is a separate reward-consensus round. Each node emits a `RewardProposal { epoch, node_id, rewards_hash }`. `compute_rewards_hash()` (`clock.rs:582-604`) is a deterministic SHA-256 over that epoch's sorted work-input keys (`mine:`, `storage_beat:`, `sensor_commit:`, `tracker_sighting:`, `infer_verify:`, `service_beat:`, `mempool_beat:`). When >51% of registered clocks propose the same hash, a `FinalizedEpoch` fires → an `EpochFinalize { rewards_hash, state_root, quorum, sealed_by }` entry is applied. `state_root` = balance/state merkle root; two honest nodes on the same entries must produce identical `rewards_hash` **and** `state_root` — divergence signals a non-determinism bug. [SPEC'D] `docs/CONSENSUS.md:88-135`. (Note: `docs/CONSENSUS.md` uses stale `state_root = full_state_hash()`; code uses `balance_merkle_root()`, `main.rs:519`.)

### Clock node registration & eligibility [BUILT] `clock.rs:619-682`

Eligibility is **pool-driven / FIFO staking**, not a mandatory explicit register tx: a node is in quorum when its aggregated `role_stake:clock:{node}:{staker}` (self + backers) ≥ `clock_min_stake` (5 HONE). Earliest first-stake epoch gets slot priority. Legacy `clock_reg:{node}` entries are honored for back-compat and pubkey caching. Slashed nodes (stake zeroed) are excluded.

- **Entry:** `ClockNodeRegister { node_id, stake, epoch, pubkey, signature }` — writes `clock_reg:{node_id}` with stake, `registered_epoch`, and `pubkey` (the ed25519 key required later for slash verification). `entry.rs:340-349`, `chain.rs:2942-2996`
- **Pool:** clock → `ClockReward` (Layer D).

### ClockReward — Layer D infrastructure base [BUILT] `main.rs:1438-1506`, `emission.rs:176-182`

`ClockReward` is the **infrastructure base** in the 4-layer model — a tiny, era-scaled per-epoch payment that fires every sealed epoch independent of Layer B pool activity. Base = `clock_reward_at(epoch)` = `CLOCK_REWARD_HUNITS (10_000_000 hunits = 0.001 HONE) << era` — doubles each era to hold constant daily income as epoch duration grows. `emission.rs:134,179-181`

Per-sealer amount is **uptime-scaled**: a sliding window (`CLOCK_UPTIME_WINDOW = 100` epochs) tracks seals/epochs per node (`clock_uptime:{node}`). New nodes get full reward for the first `CLOCK_UPTIME_MIN_EPOCHS = 10`; steady-state multiplier is `500 + 500×seals/epochs` (0.5×–1.0×, millipct). Missed seals leak one window point. `main.rs:1442-1506`. Emitted as `ClockReward { node_id, amount, epoch }` per node in `signing_clocks`; routed through `distribute_role_backer_reward("clock", …)` so backers share. `chain.rs:2142-2188`. The mandatory **2% reserve split** (1.5% recycle / 0.4% testnet / 0.1% treasury) is also Layer D and always fires. `main.rs:1418-1436`.

### Bootstrap grace period [BUILT] `chain.rs:2147-2188, 2942-2996`, [SPEC'D] `docs/CLOCK_BOOTSTRAP_GRACE.md`

HONE is pure PoW — **no account is funded at genesis** — creating a deadlock: 0 balance ⇒ no one can post the 5-HONE clock stake ⇒ no seals ⇒ chain never advances. Fix: for the first **100_000 epochs** (`CLOCK_BOOTSTRAP_GRACE_END_EPOCH`, ≈34.7 days):

- **Register at zero stake.** `E ≤ GRACE_END`: `ClockNodeRegister` accepted with any offered stake (usually 0), no minimum enforced, no debit. `chain.rs:2961-2968`
- **Stake builds from earnings.** While a clock's recorded stake < min inside the window, each `ClockReward` is routed **into its clock stake** (capped at the minimum) instead of spendable balance; spillover above the cap goes to balance. `chain.rs:2153-2183`
- **Self-terminating & deterministic.** `GRACE_END` is a fixed constant from block 0 (part of consensus, wall-clock-independent). After grace: normal min-stake enforced + debited; rewards flow to balance. Block-0 hash unaffected (apply-time logic only). `docs/CLOCK_BOOTSTRAP_GRACE.md:73-101`. Tests: register-at-0 pre-grace, min enforced post-grace, reward-builds-to-min-then-spills. `chain.rs:4250-4367`

### Double-sign slashing [BUILT] `chain.rs:2999-3062`, [SPEC'D] `docs/CONSENSUS.md:102-119`

**Entry:** `ClockDoubleSignEvidence { submitter, offender, epoch, seal_hash_a, timestamp_a, sig_a, seal_hash_b, timestamp_b, sig_b, signature }`. `entry.rs:354-368`

Any submitter can slash a clock that signs two conflicting seals in one epoch. Guarantees:
- `seal_hash_a != seal_hash_b` (identical hashes prove nothing). `chain.rs:3004-3007`
- Replay guard `dbl_slash:{offender}:{epoch}` — one slash per (offender, epoch). `chain.rs:3008-3012`
- **Both** seal signatures verified via `verify_clock_seal_sig` against the offender's *registered* pubkey over `"seal:{epoch}:{seal_hash}:{node_id}:{timestamp}"` before any stake moves — framing is cryptographically impossible. `chain.rs:3022-3041`, `clock.rs:755-787`

**D7 distribution** of the offender's registered stake: 10% → `submitter` (bounty), 10% → `__legal__`, remainder (~80%) → `__recycle_fund__`. Offender `clock_reg` stake zeroed (slashed nodes excluded from future quorum), `slashed_epoch` + pubkey retained for audit. `chain.rs:3043-3057`

### Entropy [BUILT] `clock.rs:689-716`, [SPEC'D] `docs/CHAIN_ENTROPY.md`

Per-epoch entropy = `SHA-256(XOR of winning seal hashes)`, stored at `epoch_entropy:{epoch}`. `clock.rs:689-703`, `main.rs:460-469`. Used as deterministic public randomness — e.g. storage Merkle-range challenges `sha256("{seal_hash}:{node_id}:{epoch}")` any peer can recompute. `clock.rs:712-716`.

Separately, **chain entropy** (dormancy → liveness rewards) is code-ready but **gated off**: `LIVENESS_REWARDS_ENABLED = false`, `ENTROPY_DECAY_ENABLED = false` until docs ship + governance vote. `LivenessProof` (free per-epoch heartbeat, 50% of entropy pool to live wallets, 50% recycle) and `EntropyWitness` entries exist. [SPEC'D] `docs/CHAIN_ENTROPY.md`, `entry.rs:1719-1741`. [ASPIRATIONAL] VRF-based Stage-2 epoch entropy (Phase 7).

### Hardline: no local submission without peers [BUILT] `api.rs:2028-2037, 2109-2116`

**A node with zero peers MUST NOT accept or apply any user-submitted entry.** A disconnected node that applies locally silently forks — its state diverges with no reconciliation path, and the user sees "confirmed" tx the network never saw. `apply_and_broadcast()` returns `{accepted:false, error:"not connected to network — entry rejected to prevent local fork"}` when `peer_count == 0`. Same guard is duplicated in `post_contract_deploy` (`api.rs:2109`) and `monitor.rs:81`, and lorawan-ingested entries route through the same function (`lorawan.rs:359`). Applies to **all** entry types — stakes, transfers, registrations, sensor data — no offline-mode bypass. A regression test (`api.rs:10761-10793`) fails if the check is removed. This is distinct from clock **isolation/observer mode** (`clock.rs:318-334`): a genuinely-isolated single clock may still *self-seal* its own epochs (bootstrap/single-clock branches), but it will not admit third-party user entries with zero peers.

### Upgrade path [SPEC'D] `docs/CONSENSUS.md:138-144`

- Phase 2 (current): registered-set quorum, bootstrap master, entropy, state root, double-sign slash.
- Phase 3: explicit fork-choice gossip; longest quorum-certified chain wins; 2/3 BFT threshold via governance flag.
- Phase 5/7: [ASPIRATIONAL] VDF slot assignment, missed-slot slashing, VRF entropy.

---

# Part III — The Resource Economy

> **The thesis, made vivid:** every physical resource a machine can offer — a token generated, a byte retained, a reading sensed, a packet relayed, a container-hour hosted — is a **pool** that gets paid from network state every epoch. There is no privileged resource. Each chapter below names its **reward pool** and **entry types**, and its pay is always `gated_pool × util_p / Σ util` — priced live against everything else the network did that epoch. Inference is not the network; it is one chapter of it.

## Chapter 1 — Inference & Mining

**Reward pool:** inference/mining → **MineReward** (Layer B). Also earns job escrow (worker 80%). **Status:** [BUILT], with several safety flags OFF pending activation.

Compute contributors are paid from the Layer-B activity budget exactly like every other pool: pool share = `(utilization/Σ utilization) × activity_budget`, where inference utilization normalizes against `CALIBRATION_INFERENCE = 10_000` weighted score-points/epoch [BUILT `emission.rs:355`]. Two on-ramps: the **mining path** (a node runs a model every epoch and submits `Mine`) and the **inference job marketplace** (a real requester posts a paid job a worker bids on). They are wired together: an unlinked `Mine` is a warm-up beacon; a `Mine` linked to a board-verified job earns full weight.

### Entry types

| Pool / entry | Direction | Where |
|---|---|---|
| `Mine` | user (weight NORMAL) — per-epoch compute beacon | [BUILT `entry.rs:279`] |
| `MineReward` | system — clock-emitted Layer-B payout to miners | [BUILT `entry.rs:311`, `main.rs:1932`] |
| `InferenceJobPost/Bid/Award/Complete/Commit/Verify/Claim/ReviewVote/Pay/Cancel` | marketplace state machine (escrow-funded) | [BUILT `inference.rs`, `entry.rs:462`] |
| `InferenceVerifyClaim` | verifier requests encrypted (prompt+result) | [BUILT `entry.rs:528`] |
| `VerifierReward` | system — Layer-B payout to verifiers by verification activity | [BUILT `entry.rs:634`] |
| `ModelManifestPublish / ModelUsageAttest / ModelAnalysisReward` | model registry | [SPEC'D `docs/MODEL_REGISTRY_PROTOCOL.md`] |

Escrow flow: all job funds sit in `RECYCLE_FUND_ACCOUNT` from post → pay; unused fee **recycles, never burns** [BUILT `inference.rs:323,798`].

### Mining path (`Mine` → `MineReward`)

Miners no longer self-award or produce blocks. Each epoch the miner loop runs the model **itself** through the embedded engine and submits a `Mine`; clock nodes emit `MineReward` pro-rata after seal quorum [BUILT `miner.rs:1-13,116`, `main.rs:1932`].

- **work_value** = `output_tokens` generated that epoch; `output_hash` = SHA-256 of the output text — a **binding commitment, not a reproducibility proof** [BUILT `miner.rs:3-5,226`].
- **Score** = `output_tokens × hw_tier_weight × model_weight × stake_weight` [BUILT `emission.rs:332`, `main.rs:1579`].
  - `hw_tier_weight`: phone 1, cpu 4, gpu-consumer 8, prosumer 16, server 32 [BUILT `emission.rs:301`].
  - `model_weight`: parsed from model id — 0.5B→1, 7/8B→6, 70/72B→40 [BUILT `emission.rs:315`].
- **Job linkage & grace (D8)** [BUILT `main.rs:1568-1611`, `emission.rs:444-452`]:

| Mine kind | In grace (epoch < 259_200 ≈ 90d) | After grace |
|---|---|---|
| Linked, job `Verified`/`Paid` | full score | full score |
| Linked, job pending quorum | ×`MINE_GRACE_REWARD_BPS` (20%) | dropped (0) |
| Unlinked (benchmark) | ×20% | dropped (0) |

During grace the protocol posts `MINE_GRACE_BENCHMARK_JOBS_PER_EPOCH = 5` synthetic jobs from `__testnet_fund__` so miners have real demand [BUILT `emission.rs:452`, `main.rs:2078`]. Legacy `HONE_MINER` env path still exists; new operators use `HONE_WORKER` (marketplace) [SPEC'D `docs/MINER_GUIDE.md:33-37`].

### Inference job marketplace

State machine [BUILT `inference.rs`]:

```
Posted → Awarded → Completed → Verified → Paid          (happy)
                            ↘ Rejected → Paid            (board rejects, worker 0)
                            ↘ Disputed → Claimed → Reviewed → Paid   (worker contests)
                                       ↘ (no claim in window) → NoFee → Paid
Posted/Awarded → Cancelled/Expired                       (deadline daemon)
```

- **Post**: requester debits `max_fee` into escrow; `min_verifiers` auto-scales with active board size (1/3/5 at `VERIFIER_ACTIVE_THRESH_3=5`, `_5=15`) [BUILT `inference.rs:172-180,326`]. Self-dealing blocked: same `node_fingerprint` can't bid on own job [BUILT `inference.rs:107-110`].
- **Bid** (`worker`|`verifier`), **Award**: best bid = `max(reputation/fee)`; ≤3 verifiers/job [BUILT `inference.rs:894,423`].
- **Complete**: worker submits `result_hash`, `latency_ms`; staleness gate rejects >3 epochs past deadline [BUILT `inference.rs:458`].
- **Pay** splits (bps of fee) [BUILT `emission.rs:139-147`]:

| Path | worker | verifiers | reviewers | recycle |
|---|---|---|---|---|
| Happy (approved) | 8000 | 1500 | — | 500 |
| Disputed→worker wins | 7000 | 1000 | 1500 | 500 |
| NoFee/Rejected | 0 | 1000 | 1500 | 500 |

Remainder of `max_fee` refunds to requester [BUILT `inference.rs:973`].

### Verifier board — "check REALNESS, not correctness"

- **Reputation**: nodes start `score=5000`/10000; `completion_rate × (50 + latency_factor)` [BUILT `inference.rs:140-166`].
- **Board quorum**: verdicts ∈ {`approved`,`rejected`,`review_required`}; weights NORMAL=100, REVIEW=101 (breaks ties → dispute) [BUILT `inference.rs:185-223`]. Resolves early on unbeatable lead.
- **Verification philosophy**: verifiers get the (prompt+answer) encrypted to their **memo public key** and judge "is this real work?", NOT whether the answer is right — the reasonable-not-right doctrine [SPEC'D `docs/reward-model.md:96-97`, `docs/CHAIN_CONSTANTS.md:97`].
- **value_score** = `output_tokens × hw_tier_weight × model_weight × complexity_factor` (1/2/4, verifier-assessed, bounded); feeds both `VerifierReward` Layer-B weight and the miner's epoch score [BUILT `entry.rs:554-556`, `inference.rs:643-648`].
- **Commit-reveal (T4-2)**: `InferenceJobCommit` = `sha256(verdict|salt)` before reveal; stops verdict-copying. Flag `INFERENCE_COMMIT_REVEAL_ENABLED = false` [BUILT `inference.rs:486-528` — currently OFF].
- **Random assignment (D9)**: `sha256(epoch_entropy|job_id|verifier)[0] < VERIFIER_THRESHOLD_BYTE (51)`; flag `VERIFIER_ASSIGNMENT_ENABLED = false` [BUILT `inference.rs:579-589` — OFF].
- **Anti-rubber-stamp**: leaky-bucket approval-rate over 100 epochs; ≥`VERIFIER_RUBBER_STAMP_BPS (9500)` halves vote weight, ≥`VERIFIER_SUSPEND_BPS (9900)` suspends 20 epochs [BUILT `inference.rs:665-717`].
- **Dissenter slashing**: on pay, verifiers who voted against consensus are slashed their would-have-earned share → recycle [BUILT `inference.rs:817-825,933-955`].

### Dispute → human review

Board `review_required` → `Disputed`. Worker/requester files `InferenceJobClaim` within `CLAIM_WINDOW_EPOCHS=20`; `MIN_REVIEW_VOTES=3` human `InferenceReviewVote`s decide `Reviewed` vs `Rejected`. No claim in window → `NoFee` (verifiers paid, worker 0, requester refunded) [BUILT `inference.rs:719-775,286-297`].

### Layer-C anti-circular fee boost

Only `worker_amount > 0` on an **approved** job records a `fee_flow:{epoch}:{job_id}`; next epoch's Layer-C boost is `min(verified_fee_vol × FEE_BOOST_RATE, MAX_BOOST)` on **net new-capital flow only**, so circular self-payments cancel [BUILT `inference.rs:839-850`].

### Embedded candle engine + external path

The node runs the model **in-process** — `inference_engine::chat` is the single entry point (fixed the ~6 scattered Ollama call sites that 503'd at launch) [BUILT `inference_engine.rs:1-21`; SPEC'D `docs/INFERENCE_EMBED_SPEC.md`]. Backend chosen once at startup, priority [BUILT `inference_engine.rs:64-89`]:

1. `INFERENCE_URL` (or legacy `OLLAMA_URL`) → external OpenAI/Ollama-compatible server (the vLLM GPU-throughput tier).
2. else built with `inference-embedded` → in-process **candle GGUF**.
3. else → `None` (relay-only node).

Embedded engine [BUILT `inference_engine.rs:154-495`]:
- **Model-agnostic, no hardcoded default, never auto-downloads** — operator selects a GGUF via `HONE_MODEL`; unavailable until enabled [BUILT `inference_engine.rs:161-222`].
- **Multi-arch dispatch** by GGUF `general.architecture`: `llama`/`qwen2`/`qwen3` (fixes the qwen "wiring bug") [BUILT `inference_engine.rs:265-404`].
- Greedy/argmax decode, KV-cache incremental; CPU default, GPU opt-in via `HONE_INFER_DEVICE=cuda[:N]` (CPU/CUDA not bit-identical → mining stays CPU) [BUILT `inference_engine.rs:288-494`].
- Tokenizer must sit beside the GGUF, no auto-download [BUILT `inference_engine.rs:406-415`].
- **Throttle-aware**: per-token sleep when `throttle::throttle_percent() < 100` [BUILT `inference_engine.rs:466-476`].

Miner reuses this same engine (never a separate Ollama daemon): `run_inference_prompt` caps at 64 tokens [BUILT `miner.rs:52-54,202-235`].

### Model registry [SPEC'D `docs/MODEL_REGISTRY_PROTOCOL.md`]

Models keyed by **content hash** (sha256 of weights), not name. First valid `ModelManifestPublish` wins; `ModelUsageAttest` (system, only on settled paid jobs) increments usage; `ModelAnalysisReward` pays the analyzer a capped, usage-weighted share **off the existing pool (no new issuance)**. Anti-gaming: manifest fraud slashable, fake usage impossible, sybil analysis earns nothing. Not yet built as entry types in `entry.rs`.

### Aspirational / open

- Real off-chain job **input fetch** (hone-fs/D1): the miner currently derives a deterministic prompt from `job_id`+`input_hash` rather than fetching the actual input [ASPIRATIONAL `miner.rs:164-171`].
- Cross-node **inference determinism** (CPU/GPU reduction-order) [ASPIRATIONAL `inference_engine.rs:288-303`].
- Streaming embedded generation (SSE) [ASPIRATIONAL `docs/INFERENCE_EMBED_SPEC.md:155-163`].

---

## Chapter 2 — Sensors & IoT (Verasens)

**Reward pool:** sensors → **SensorReward** / **GatewayRewardSplit** (Layer B). Also earns **SensorDataPurchase** premium. **Status:** chain-side economics [BUILT]; the Verasens protocol crate itself is [SPEC'D] "early design" (`verasens/README.md`); premium market's richer design is [SPEC'D]/[ASPIRATIONAL] on the deprecated Node path.

Phones and IoT devices contribute readings, cellular coverage, and GNSS/environmental data and are **paid from network state**. Sensor pool share = `(sensor_utilization / Σ utilization) × activity_budget`, `sensor_utilization = Σ sensor_score / CALIBRATION_SENSOR` [BUILT `main.rs:1640-1692`, `emission.rs:365`].

### Entry types

| Concern | Entry type | Reward pool | Where |
|---|---|---|---|
| Sensor reward payout | `SensorReward` | **Sensor (Layer B)** | [BUILT `entry.rs:628`, `chain.rs:2218`] |
| Gateway-relayed split | `GatewayRewardSplit` (60/40) | **Sensor (Layer B)** | [BUILT `entry.rs:852`, `main.rs:1944-1963`] |
| Batch data commit (scoring input) | `SensorDataCommit` | — | [BUILT `entry.rs:834`, `chain.rs:1395`] |
| Single reading | `SensorReading` | — (bootstrap/legacy) | [BUILT `entry.rs:418`, `tx.rs:468`] |
| Coverage measurement | `CoverageReport` | — (scoring input) | [BUILT `entry.rs:442`, `chain.rs:1291`] |
| Premium data sale | `SensorDataPurchase` | direct owner/recycle split | [BUILT `entry.rs:1064`, `chain.rs:2263`] |
| Registration / vouch | `SensorRegister`, `SensorKeyRegister`, `DeviceKeyRegister`, `SensorVouch` | — | [BUILT `entry.rs:809-867`] |
| Anti-sybil device ownership | `DeviceClaimStake`, `DeviceClaimUnstake` | — | [BUILT `entry.rs:679-695`, `chain.rs:2306`] |
| Device yield staking | `DeviceYieldStake`, `DeviceYieldUnstake` | shares overbid premium | [BUILT `entry.rs:870-886`, `chain.rs:2337`] |

### Type-aware value model — `sensor_score(reading_count, sensor_type)`

Raw reading count is meaningless without a value model. Four declared models (device declares `sensor_type` at commit; registry enforces consistency) [BUILT `emission.rs:273-288`]:

| `sensor_type` | Scoring | Rationale | Example |
|---|---|---|---|
| `continuous` | `isqrt(readings × 100)` | sqrt-diminishing to stop spam | temp, humidity, power |
| `event` | `min(readings,20) × 100` — **cap 20** | each reading individually valuable | GPS commit, seismic trigger |
| `sampled` | `min(readings,60)×30 + min(readings-60,940)×5` | reduced rate past 60 | air quality, CO2 |
| `pulse` | `0 if none; else 200 + min(readings-1,10)×10` | presence/uptime proof (**+10 cap**) | heartbeat, occupancy |
| unknown | `isqrt(readings × 10)` | conservative fallback | `custom`, `android-*` |

Reading count is hard-capped at **10,000/epoch** at commit to bound state [BUILT `chain.rs:1399`, `emission.rs:284`].

**Location boost:** registered non-empty `location` earns **1.3×** (`SENSOR_LOCATION_BOOST_BPS = 13_000`); after `SENSOR_LOCATION_REQUIRED_EPOCH` (currently `0` = disabled, planned mainnet month 3) unlocated sensors are **skipped** [BUILT `main.rs:1657-1663`, `emission.rs:574-579`].

**Calibration & scarcity:** `CALIBRATION_SENSOR = 5_000`; critical-mass gate `CRITICAL_MASS_SENSOR = 20` — below 20 distinct owners pays `count/20`, remainder recycles [BUILT `emission.rs:365,388`; `main.rs:1905-1929`].

### Payout flow (per epoch, at seal)

1. Scan `sensor_commit:{epoch}:*` → per-owner `sensor_score` sum (with location boost) [BUILT `main.rs:1647-1669`].
2. Merge `coverage_report:{epoch}:*` scores into the same by-owner map [BUILT `main.rs:1673-1690`].
3. Each owner's share = `sensor_pool × owner_score / total_score` [BUILT `main.rs:1939-1943`].
4. If a `gateway_account` was recorded → emit `GatewayRewardSplit` **60% sensor / 40% gateway** (`SENSOR_BPS=6_000 / GATEWAY_BPS=4_000`); else plain `SensorReward` [BUILT `main.rs:1944-1963`].

### Coverage mapping (cellular dead-spot vertical) — `CoverageReport`

Crowdsourced signal maps sellable to telcos. `signal_dbm = None` (or `< -120 dBm`) = **dead spot**, scored higher (rarer/more valuable) [BUILT `entry.rs:442-458`, `chain.rs:1291-1349`].

| Constant | Value | Meaning |
|---|---|---|
| `COVERAGE_SCORE_SIGNAL` | 100 | normal signal report |
| `COVERAGE_SCORE_DEAD_SPOT` | 300 | dead-spot report (3×) |
| `COVERAGE_CORROBORATION_BONUS_BPS` | 15_000 | 1.5× when corroborated |
| `COVERAGE_CORROBORATION_MIN_REPORTERS` | 3 | distinct devices to corroborate |
| `COVERAGE_MAX_CORROBORATING_REPORTERS` | 10 | **Sybil cap** — extra reporters can't tip corroboration |
| `COVERAGE_GRID_RESOLUTION` | 1000 | 0.001° ≈ 100 m grid cell |
| `COVERAGE_MAX_REPORTS_PER_EPOCH` | 200 | per-reporter anti-spam cap |

Cells quantized to a 100 m grid × carrier (`carrier_mcc_mnc`, E.212 `{MCC}-{MNC}`). The prompt's "capped 10/sensor" maps to two real caps: the **pulse +10-extras cap** and the **coverage 10-reporter corroboration cap** [BUILT `emission.rs:589-614`, `chain.rs:1301-1348`].

### Premium data market — `SensorDataPurchase`

A buyer pays a fee to purchase a committed batch (`batch_hash`). Settlement is a **direct on-chain split: 80% owner / 20% recycle** (the doc's storage-contract cut is not yet wired). Nonce-checked, active-key signed, balance-gated [BUILT `entry.rs:1064-1075`, `tx.rs:1027-1040`, `chain.rs:2263-2273`].

> A fuller monetization design — quote/query/analyze endpoints, escrow settlement, 70/20/10 sensor/protocol/recycle split (protocol 50/50 to founders), privacy-preserving GPS (metro default, opt-in raw), enterprise rate cards, combined data+inference `/v1/sensor-data/analyze` — is [SPEC'D] on the **deprecated Node.js prototype** (`docs/SENSOR_DATA_MONETIZATION_PLAN.md`), [ASPIRATIONAL] on the Rust node.

### Identity, keys & anti-sybil

- **Device keys:** `SensorKeyRegister` / `DeviceKeyRegister` bind a per-device signing pubkey (optional `hardware_hash`) to an owner. `SensorDataCommit` is device-key signed, but `signed_by` must equal `owner`, verified against `sensor:{id}` registry (VEC-3) [BUILT `entry.rs:819-867`; `chain.rs:1406-1418`].
- **Bootstrap-compatible signing:** keyless-owner `SensorReading` may be unsigned; enforced once a posting key exists [BUILT `entry.rs:425-434`, `tx.rs:468`].
- **DeviceClaimStake (anti-sybil):** stake HONE to claim a hardware serial; first staker wins; challenger must **overbid ≥ 1.5×**; displaced principal returned, premium split to device yield-stakers, remainder recycled; unstake is unbonding (T3-3) [BUILT `chain.rs:2306-2380, 4936-4978`].
- **DelegationGrant** capability `"SensorSubmission"` lets a hot key submit on an owner's behalf [BUILT `entry.rs:1536`].

> **Not inference-only:** an environmental sensor, a LoRa gateway, or a phone mapping cellular dead spots earns every epoch from `SensorReward`/`GatewayRewardSplit` with **no inference involved**, purely from `sensor_score`, location boost, gateway relay, coverage corroboration, and the dynamic pool budget.

---

## Chapter 3 — Storage (HDD) & Data

**Reward pool:** storage → **StorageReward** (Layer B) + **contract access fees**. Earning entries: `StorageHeartbeat`, `BlobStore`, `HiveReplicaCommit`/`HiveReplicaVerify`, `LinkGitStorageExtend`, `SensorDataPurchase` (storage rate); phone path `phone_storage::apply_proof`. **Status:** [BUILT] core; BlobStore fee-collection + erasure coding [ASPIRATIONAL].

Any node that dedicates disk proves it holds bytes each epoch and is paid from the storage pool plus contract access fees. Phones participate too (`phone_storage.rs`).

### Two income streams

| Stream | Basis | Source |
|---|---|---|
| **Baseline** (Layer B) | `bytes_proven × query bonus (up to 2×) × tier` | storage pool share of activity budget |
| **Contract access fees** | `BlobStore` fee, LinkGit push/pull serve, `SensorDataPurchase` storage rate | direct fees, not the pool |

Baseline is fully dynamic: `util_storage = storage_total / CALIBRATION_STORAGE`; pool = `(util_storage / Σ util) × activity_budget` [BUILT `main.rs:1824,1893`]. No static percentage.

### StorageHeartbeat — proof of retained bytes [BUILT]

Emitted each epoch when `HONE_STORAGE=true` [BUILT `main.rs:23,745`]. Fields [BUILT `entry.rs:896`]: `bytes_proven`, `query_count` (drives query bonus), `challenge_response: Option<MerkleRangeProof>`, `tier: Option<u8>` (1/2/3).

**Challenge/proof (T4-1, D10)** [BUILT `chain.rs:475-519`]:
- Challenge deterministic: `challenge_hash = sha256("{prev_seal_hash}:{node_id}:{epoch}")` — can't be precomputed/shared.
- Chunk index = `challenge[0..8] mod num_chunks`; chunk size `STORAGE_CHALLENGE_CHUNK_BYTES = 1 MiB` [BUILT `emission.rs:571`].
- Prover returns `leaf_hash` + `proof_nodes`; node walks the Merkle path and checks it reconstructs `merkle_root` [BUILT `chain.rs:504-518`].
- **Anti-inflation (T2-4):** on valid proof `effective_bytes = total_chunks × 1 MiB`, NOT self-declared `bytes_proven` — you can only claim bytes you can Merkle-prove [BUILT `chain.rs:1362-1379`].
- **No-proof penalty:** missing/invalid → score × `STORAGE_PROOF_NO_PROOF_BPS = 2000` = **20% of normal** [BUILT `emission.rs:566`, `main.rs:1625`].

### Score → StorageReward [BUILT `main.rs:1613-1638`]

```
bonus  = bytes × min(query_count, 100) / 100     // up to +1× → 2× total
raw    = bytes + bonus
score  = raw            if proof_valid
       = raw × 20%      if no valid proof
effective_tier = 3 if tier==3 & ≥100 GB
                 2 if tier==2 & ≥10 GB
                 else 1
tier_mul       = {3→5×, 2→2×, 1→1×}
final = score × tier_mul
```

Tiers pay **1× / 2× / 5×**, but only if `bytes_proven` actually clears the threshold [BUILT `main.rs:1628-1637`]. `StorageReward` distributed pro-rata across `storage_nodes` from `storage_pool` [BUILT `main.rs:1934-1936`]. Scarcity gate: below critical mass, pool pays a reduced fraction, remainder recycles [BUILT `main.rs:1553,1915`].

### BlobStore [BUILT partial]

`BlobStore { cid, uploader, size_bytes, epoch, fee }` [BUILT `entry.rs:698`] records a content-addressed blob and its access fee. Currently accepted with no balance mutation at the base layer (`chain.rs:1352-1356`); full fee-collection wiring is [ASPIRATIONAL] (`reward-model.md:118`).

### Phone storage [BUILT]

Phones are full storage nodes. `phone_storage::apply_proof` verifies `submitted_hash == sha256(device_id | inner | account | epoch_le8)`, enforces single-use, credits a flat `PHONE_STORAGE_REWARD_HUNITS = 250` hunits [BUILT `phone_storage.rs:12,18-63`] — a simpler flat-rate path than the Merkle-challenge desktop path.

### Hive external replica — second storage domain [BUILT chain support]

A storage operator mirrors HONE-FS blobs into Hive `custom_json` as an **external decentralized replica** — a *separate* storage domain, never local disk [BUILT `entry.rs:918-962`, `hive_replica.rs`; SPEC'D `docs/BTCPC_FS_HIVE_EXTERNAL_REPLICA_PLAN.md`].

| Entry | Who | Effect |
|---|---|---|
| `HiveReplicaCommit` | storage node | records Hive account/tx, CID, `merkle_root`, `bytes_replicated`, `replica_kind`, `confirmations`. **Earns nothing alone.** |
| `HiveReplicaVerify` | independent verifier | fetches the Hive tx, matches commit, passes challenge → creates `storage_beat:{epoch}:{node_id}:hive` reward slot |

Two reward slots per operator: local + `…:hive`. **Replica-kind weights** (of verified bytes): `full 75%`, `chunk 50%`, `parity 30%`, `manifest 5%` (cap 1 MiB). Hive slot capped at 100 GiB score/node/epoch, forced tier 1. Anti-gaming: no self-verify, ≥20 confirmations, one event per ref per epoch [BUILT `hive_replica.rs`]. The `btcpc-hivefs-adapter` sidecar (Phases 2–3) is [ASPIRATIONAL].

### Erasure / replication [ASPIRATIONAL]

`replica_kind` (`full | chunk | parity | manifest`) encodes the erasure/replication model at the entry level [BUILT schema `entry.rs:929`]. Actual erasure-coding shard generation, replication-factor targeting, and domain-diversity for premium storage classes are stated goals not yet built [SPEC'D].

### LinkGit storage [BUILT adjacent]

`LinkGitStorageExtend { cids, keep_until_epoch, fee }` pays to keep prunable objects [BUILT `entry.rs:1138`, `tx.rs:2615`]; `LinkGitPruneProof` earns for confirmed GC. These feed the linkgit pools, separate from the storage pool.

---

## Chapter 4 — Network Services (Mempool / Service / Runtime / Gateway)

Beyond inference, storage, and sensors, HONE pays four "network service" resource classes. Two (**mempool relay**, **service hosts**) are first-class Layer B activity pools. One (**decentralized runtimes**) is paid per-job from fee escrow, not a pool. One (**gateways**) earns only as a downstream split of another pool. Every earner is gated by the no-peers hardline and the 4-layer cap.

### Resource map

| Service | Register/liveness entry | Reward entry & pool | Basis | Layer B pool? |
|---|---|---|---|---|
| Mempool relay | `MempoolOperatorRegister`, `MempoolHeartbeat` | **MempoolReward** | latency-weighted relay throughput | **Yes** [BUILT] |
| Service host | `ServiceHeartbeat` | **ServiceReward** | active container-hours | **Yes** [BUILT] |
| Decentralized runtime | `RuntimeRegister/Deploy/JobEnqueue/Claim/Attest/Challenge/Slash` | **RuntimeReward** (defined, never emitted) + per-job escrow | 80% of job fee at attest | **No** — per-job escrow [BUILT escrow; RuntimeReward SPEC'D-only] |
| Gateway (LoRa/IoT) | `GatewayHeartbeat` | **GatewayRewardSplit** | 40% of relayed sensor's `SensorReward` | **No** — downstream split [BUILT] |

### 1. Mempool relay (staked relay node type)

A mempool operator stakes HONE and earns for propagating entries fast [BUILT `entry.rs:656-673`].
- **`MempoolOperatorRegister`** stakes HONE; validation requires `signed_by == operator`, key, nonce, active-key sig, balance ≥ amount [BUILT `tx.rs:1064`]. The entry comment references a `MEMPOOL_MIN_STAKE` minimum, but **no such constant exists and no minimum is enforced** — only `balance ≥ amount` [MEMPOOL_MIN_STAKE ASPIRATIONAL — doc-comment only, `entry.rs:658`].
- **`MempoolHeartbeat`** `{ propagation_latency_ms, entries_relayed }` — the live daemon hard-codes `propagation_latency_ms = 200` and reports a real `entries_relayed` counter [BUILT `main.rs:1021, 2287-2342`].
- **Slashing:** censorship / double-inclusion / fee front-running are listed as slashable, but no evidence entry or handler exists [ASPIRATIONAL `entry.rs:654-655`].

**Reward (MempoolReward, Layer B):** score = `entries_relayed × 1000 / max(latency_ms,1)` [BUILT `emission.rs:292`]. Zero-relay heartbeats score zero and drop. Normalizes against `CALIBRATION_MEMPOOL = 10_000`; scarcity `CRITICAL_MASS_MEMPOOL = 3`. System-only (`tx.rs:1501`). Distributed at seal [BUILT `main.rs:1971`].

### 2. Service hosts (container-hours)

- **`ServiceHeartbeat`** `{ node_id, epoch, container_hours }` — the only service entry; posting-signed, emitted when `HONE_SERVICE=true` [BUILT `entry.rs:963-970`, `tx.rs:924-932`, `main.rs:759`]. There is **no** ServiceRegister / bond / slashing — a host simply heartbeats claimed hours.
- **Reward (ServiceReward, Layer B):** score = self-reported `container_hours`; normalized against `CALIBRATION_SERVICE = 24`; scarcity `CRITICAL_MASS_SERVICE = 3`; system-only; distributed at seal [BUILT `main.rs:1704-1710, 1968`]. "Also earns service usage fees" is doc-stated but **no on-chain service-fee mechanism is wired** [ASPIRATIONAL `reward-model.md:55`].

### 3. Decentralized runtimes (durable jobs, leases, attest/slash)

A crypto-verifiable, slashable hosting layer for HTTP services / workers / stateful sessions (OCI + WASM). v1 is explicitly "not fully trustless yet — signed attestations + challenge proofs + slashing" [SPEC'D `docs/PLAN_DECENTRALIZED_RUNTIMES_V1.md`]. Full lifecycle BUILT as entries, state maps, API routes (`api.rs:353-362`):

| Entry | Role | Effect [BUILT `entry.rs:971-1062`, `tx.rs:935-1004`] |
|---|---|---|
| `RuntimeRegister` | anchor manifest CID, lock bond | bond ≥ `RUNTIME_MIN_BOND` (5 HONE) |
| `RuntimeDeploy`/`RuntimeUndeploy` | place/remove runtime | owner-signed |
| `RuntimeJobEnqueue` | durable job, escrow fee | `fee` → `__runtime_job_escrow_{job_id}__` (`chain.rs:1508`) |
| `RuntimeClaim` | host takes a lease | one active lease per job; TTL/failover |
| `RuntimeAttest` | signed proof of execution | releases escrow; `runtime_sha` must equal `manifest_cid` (`chain.rs:1581`) |
| `RuntimeChallenge` | peer contests attestation | evidence CID |
| `RuntimeSlash` | penalize proven fault | clock/governance-submitted |

**Payment is per-job escrow, NOT a Layer B pool.** At `RuntimeAttest` the job fee splits `RUNTIME_FEE_HOST_BPS = 8000` (80%) → host, `RUNTIME_FEE_RECYCLE_BPS = 2000` (20%) → recycle [BUILT `chain.rs:1622-1633`, `emission.rs:407-408`]. **The `RuntimeReward` system entry is defined with a host-crediting apply path (`chain.rs:1711-1714) but is NEVER emitted — there is no runtime pool in the Layer B seal computation; runtime does not appear in `main.rs`'s `total_util`.** Hosts are paid entirely from user job fees, not emission [BUILT escrow; RuntimeReward emission = ASPIRATIONAL — dead system entry].

**Not built:** deterministic trustless replay; challenge→auto-`RuntimeSlash` adjudication; resource-proof envelope verification [ASPIRATIONAL `PLAN_DECENTRALIZED_RUNTIMES_V1.md`].

### 4. Gateways (LoRa / IoT relay)

Gateways relay sensor traffic and earn as a **cut of the sensor's own reward**, not from their own pool [BUILT].
- **`GatewayHeartbeat`** `{ gateway_id, owner, epoch }` — liveness only; sig enforced when the owner has a posting key (bootstrap-compatible); applying it records `last_heartbeat_epoch` only; **no reward on its own** [BUILT `entry.rs:888-893`, `tx.rs:474`, `chain.rs:1737`].
- **Earning path:** when a `SensorDataCommit` names a `gateway_account`, that sensor's `SensorReward` is emitted as **`GatewayRewardSplit`** — 60% (`SENSOR_BPS = 6000`) sensor / 40% (`GATEWAY_BPS = 4000`) gateway [BUILT `main.rs:1944-1958`, `chain.rs:2222`].

### Cross-cutting

- **System-only enforcement:** `MempoolReward`, `ServiceReward`, `RuntimeReward`, `GatewayRewardSplit` cannot be user-submitted [BUILT `tx.rs:94-100, 1501`]; they apply immediately on seal.
- **Backer yield:** `NodeRoleOptIn`/`NodeRoleStake` support roles `"service"` and `"mempool"` (backers earn ≤50% of the node's role rewards); no `"runtime"`/`"gateway"` role [BUILT `entry.rs:234-266`].
- **Live daemons:** only `HONE_MEMPOOL` and `HONE_SERVICE` spawn per-epoch heartbeat loops; runtime hosting is driven by external job flow via API [BUILT `main.rs:705-1028`].

---

# Part IV — Identity, Keys & Security

HONE identity is **name-primary** (`@bullship`, ENS/Hive-style — never a raw `0x` hex blob), backed by a six-role ed25519 key hierarchy, hardware anti-sybil, and cross-chain linking. Security is defense-in-depth: role separation, per-slot 2FA, an owner-level threshold, and a hard human-intent gate on transfers. Nothing in this section is a reward pool — identity/security entries carry a *fee weight* but earn no reward directly; they gate who may submit the entries that do earn.

### Account Model

An account is a claimed **name** mapping to a `keys` map (role → ed25519 pubkey), optional cross-chain commitments, and optional hardware fingerprint [BUILT `entry.rs`, `chain.rs:777`, `tx.rs:247`].

| Field | Meaning | Tag |
|---|---|---|
| `account` | claimed name; 3-digit all-numeric (000–999) reserved/rejected | [BUILT] `tx.rs:250` |
| `keys` | role→pubkey; each 64-char hex ed25519; `owner` required | [BUILT] `tx.rs:253` |
| `chain_proofs` | cross-chain commitments (easy mode auto-filled) | [BUILT] `entry.rs:106` |
| `funded_by` | name-stake funder when `name_stake_enabled` on (default OFF, free registration) | [BUILT] `tx.rs:267` |
| `machine_fingerprint` | `SHA-256(gpu_serial \| machine-id)`; blocks same-machine self-dealing | [BUILT] `api.rs:1826`, checked `api.rs:2374` |

`AccountCreate` is signed by the **owner** key over the canonical message [BUILT `tx.rs:2227`]. System accounts (`__treasury__`, `__recycle_fund__`, `STAKE_EXEMPT_ACCOUNTS`) have names but no keyed address and bypass `require_key` [BUILT `tx.rs:2142`].

### Key Roles (six-slot hierarchy)

Roles stored per-account, selected per-entry by `check_signature(..., role)` [BUILT `entry.rs:117,187`]: `"owner" | "active" | "posting" | "memo" | "hide" | "seek"`.

| Role | Purpose | Signs | Tag |
|---|---|---|---|
| owner | key rotation, 2FA policy — highest authority | `AccountCreate`, `SetKeyPolicy` owner slot | [BUILT] `tx.rs:263,1418` |
| active | value movement | `Transfer`, `Stake`, `Unstake` | [BUILT] `tx.rs:176,206,233` |
| posting | daily operational entries (mine, sensor, service, register…) | the vast majority of entries | [BUILT] `tx.rs:303+` |
| memo | encrypted messages | — | [SPEC'D] `entry.rs:101` |
| hide | private-repo owner pubkey (LinkGit) | `LinkGitRepoRegister` hide key | [BUILT] `entry.rs:1086` |
| seek | discovery/read counterpart to hide | — | [SPEC'D] `ADDRESS_SCHEME.md` |

SLIP-10 derivation is `m/44'/6942'/role'/0'` — the six roles are hardened, unlinkable children ("not a privacy chain but functionally private") [SPEC'D `ADDRESS_SCHEME.md`]. `LivenessProof` accepts a sig from *any* of the six role keys [BUILT `tx.rs:1474`].

### Addressing: Names + Typed bech32 [SPEC'D → ASPIRATIONAL]

Names are primary; typed **bech32** addresses are the machine/interop form, checksummed so wrong-type/mistyped addresses hard-fail at parse [SPEC'D `docs/ADDRESS_SCHEME.md` v0.1].

| HRP | Entity | Keyed by |
|---|---|---|
| `hh` | account/user | ed25519 account pubkey |
| `hk` | contract | code/deploy hash |
| `ht` | token | mint/derivation hash |
| `hd` | device/sensor | device ed25519 pubkey |
| `hv` | vault | vault ed25519 pubkey |
| `he` | escrow | escrow account pubkey |

`hh1…` is `bech32(existing pubkey)` — **no wallet remaking**, keys reused. Ed25519 + SLIP-10 = Ledger/Trezor/Keystone compatible by construction; the HONE Ledger app is a build, not a redesign. **Status: address module not yet built** — the doc's "Build Notes" (`hone-types::address`) are unimplemented [ASPIRATIONAL].

### Cross-Chain Linking (easy / hard mode)

An external-chain address is **never stored** — only `commitment = sha256(chain:address:nonce)` [BUILT `ChainProof` `entry.rs:15`].

| Mode | How | Trust | Tag |
|---|---|---|---|
| easy | self-asserted; all keys from one BIP-39 mnemonic | low | [BUILT] `entry.rs:21` |
| hard | user signs `hone:link:{account}:{chain}:{nonce}` with existing wallet (MetaMask/Ledger/Phantom) | cryptographic | [BUILT] `VerifyChainLink` `entry.rs:162`, `tx.rs:1455` |

Supported `sig_type`: `eth_personal_sign`, `sol_sign`; verification via `recover_chain_address_public` [BUILT `tx.rs:2208`]. Cross-chain identity binding is **private, sign-request-gated, and NOT in genesis** (`docs/CROSS_CHAIN_IDENTITY_BINDING.md`).

### 2FA & Owner Threshold

Each key slot can independently carry a 2FA policy backed by a *different* external-chain wallet [BUILT `SetKeyPolicy` `entry.rs:185`, `check_slot_2fa` `tx.rs:2170`].
- **Per-slot 2FA:** the `TwoFactor` sig covers `hone:2fa:{entry_hash}:{epoch}`, binding the factor to the exact transaction; the recovered address must have an on-chain chain proof for the account. A `Transfer` checks the `active` slot's 2FA [BUILT `tx.rs:177`].
- **OwnerAuth 3-of-4 adaptive threshold:** owner-level actions need owner key sig **+ any 2 of** {owner_2fa, corroborant active/posting sig}; absent factors lower the bar proportionally [BUILT `OwnerAuth` `entry.rs:84`; `tx.rs:1423–1448`].
- **Legacy global `TWOFA_TOKEN`** is "security theater"; per-user TOTP (`otpauth://`, 8 backup codes) is [SPEC'D `docs/totp-design.md`, post-launch]. The role hierarchy is the primary control; TOTP is additive [ASPIRATIONAL].

### Account Transfer / Recovery (never-keyless invariant)

`AccountTransfer` rotates an identity to a new owner's full key set, but **requires `AccountSetPrimary` first** — the stored primary (an existing account sharing the same posting key) receives any balance before keys rotate, proving the owner won't be stranded keyless [BUILT `entry.rs:125,144`, `tx.rs:402–419`].

### Hardware Anti-Sybil [BUILT `src/hardware.rs`]

One physical machine = one HONE account.
- **Fingerprint:** `SHA-256(gpu_serial | machine-id)`. GPU serial via `nvidia-smi`/AMD sysfs/`lspci`; machine-id via `/etc/machine-id`, `IOPlatformUUID`, Windows `MachineGuid` [BUILT `hardware.rs:34,46,100`].
- **`HardwareClaim`:** first account to claim a fingerprint wins network-wide; duplicate claims from a different account rejected. Node self-claims at boot, re-checks after every seal, logs a conflict alert on loss [BUILT `entry.rs:1761`, `chain.rs:2771`, `main.rs:322,382`].
- **`DeviceClaimStake` / `Unstake`:** stake HONE to claim a device by hardware-burned serial (IMEI/CPU/TPM EK); first staker wins; slashable for fraud [BUILT `entry.rs:679`].
- **`AmberPillMint`:** soulbound NFT, one per hardware fingerprint, grants 1.5× entry-weight multiplier [BUILT `src/amber_pill.rs`, `entry.rs:2434`].
- The fingerprint also seeds the `SecretStore` AES-256-GCM key (`HONE_SECRETS_PASSPHRASE` default = hw_fingerprint) [BUILT `secret_store.rs:33`].

### The "Never Auto-Sign Transfers" Rule

A hard **human-intent gate**: the node must NEVER autonomously sign a token transfer. A sign-request is routed to Shin or a triumvirate founder wallet (shindevlin / natoshisakamoto / josh) for review [ASPIRATIONAL as protocol enforcement; operational policy]. This is *deliberately* not a multisig mechanism: the OSS-scan proposal to replace it with a **FROST 2-of-3 founder vault was REJECTED** — FROST solves multisig mechanics but removes the human review beat, and HONE's bottleneck is *intent verification*, which FROST does not provide (`reports/research/architect-verdict-2026-07-06.md:111`).

### Private Authorization Stack (future) [SPEC'D/partial]

A chain-agnostic layer where a spend requires an approval **receipt** from a user-chosen external chain (policy chain ≠ execution chain) [SPEC'D `docs/PRIVATE_AUTH_STACK.md`]. Staged **off** by default (`HONE_PRIVATE_AUTH_ENABLED=false`). Backends in rollout order: Bitcoin signed-challenge → Lightning invoice-settlement → existing sig chains → portable zkVM (SP1/RISC0/Noir) → HONE-native ZK. Invariants: replay rejected, expiry enforced, amount/recipient/sender bound to the signed challenge, threshold met before execution. Only commitments stored. Scaffolding in `src/private_auth.rs`.

### Slashing Across Roles

All slashed stake routes to the **recycle fund** (never burned); slashing is punitive (provable misbehavior), not for being offline.

| Offense / role | Entry / mechanism | Penalty | Tag |
|---|---|---|---|
| Clock double-sign | `ClockDoubleSignEvidence` (D7) | 10% submitter / 10% legal / ~80% recycle | [BUILT] `entry.rs:354` |
| Validator double_sign/downtime/invalid_seal | `SlashValidator` + `SlashAppeal` | governed | [BUILT] `entry.rs:1840,1856` |
| Runtime host misbehavior | `RuntimeChallenge` → `RuntimeSlash` after adjudication | bond slashed | [BUILT] `entry.rs:1041,1048` |
| Verifier dissent (inference) | `dissenter_slashes` → recycle | per-job | [BUILT] `entry.rs:599` |
| Device fraudulent claim | `DeviceClaimStake` stake forfeit | stake | [BUILT] `entry.rs:678` |
| Mempool censorship/double-inclusion | slashable | — | [SPEC'D] `entry.rs:654` |
| Work-proof replay | matching proof hashes across epochs | 10× epoch reward | [SPEC'D] `SLASHING_LOGIC_REVIEW.md` (S-03) |
| Fraudulent work approval (verifier) | dispute challenge | 25% verifier stake | [SPEC'D] S-matrix |
| Slashing-evidence censorship (proposer) | gossip-timestamp vs block | 5% proposer stake | [SPEC'D] S-02 |
| Sustained miner inactivity | rolling 10-epoch rate | 0.5%/epoch below threshold, warn first | [SPEC'D] S-01 |
| Sybil verifier registration | key/IP clustering | registration stake forfeit | [SPEC'D] S-05 |

**Explicit exclusions:** storage hosts are **never slashed for absence** (absence = no payment, not punishment); clock nodes slashed only for provable timestamp manipulation, not drift within tolerance [SPEC'D `SLASHING_LOGIC_REVIEW.md`].

> **Relation to the multi-resource thesis:** identity/keys are admission control *upstream* of every reward pool — `check_signature(role)` decides who may submit the entries that Layer-B pools pay against. Hardware anti-sybil keeps per-pool utilization honest; without it, one machine could farm every pool under many names and distort the market-decided split. Slashing (recycle-routed) is the enforcement backstop.

---

# Part V — Applications

The app layer sits **on top of** the multi-resource reward economy — not beside it. Every application is a set of **native ledger entry types** (not a smart-contract sidechain), and every application that generates work either (a) feeds a **Layer-B activity pool** (LinkGit serve/build), (b) generates **Layer-C fees** from the previous epoch's approved jobs (commerce, storage, contract, sensor-purchase fees), or (c) is a pure user-to-user value transfer paying the network only via standard entry fees (Freeport orders, project bounties, contract calls, bridge ops). Consensus records **facts**, never subjective quality. Applied on **every node** at epoch seal in sha256 order via `validate_and_apply` → RocksDB. No application bypasses the no-peers hardline.

### Freeport — Sovereign Commerce [BUILT]

Censorship-resistant marketplace baked into genesis. The chain **is** the delivery channel: digital goods are encrypted to the buyer's on-chain hide key (X25519 ECDH + AES-256-GCM) and attached to `OrderFulfill` — no delivery server [SPEC'D `docs/FREEPORT_PROTOCOL.md`]. Reserved genesis accounts: `freeport` (fee authority), `freeport-escrow`.

| Entry | Signer | Effect | Status |
|---|---|---|---|
| `StoreUpdate` | posting | Create/update storefront | [BUILT] tx.rs:547 |
| `ProductCreate`/`ProductUpdate` | posting | List/modify product | [BUILT] tx.rs:548 |
| `OrderPlace` | memo/active | Lock `total_price` in `freeport-escrow`; carries buyer pubkey | [BUILT] tx.rs:633 |
| `OrderFulfill` | seek | Attach encrypted `delivery_cid` / tracking | [BUILT] tx.rs:634 |
| `OrderCancel` | hide/seek | Refund escrow to buyer | [BUILT] |
| `OrderDispute` | hide | Freeze escrow, open dispute | [BUILT] tx.rs:860 |
| `EscrowRelease` | seek (auto) / arbitrator | Release escrow to seller | [BUILT] tx.rs:637 |
| `FlashSale` | posting | Time-bounded discount | [BUILT] |

**Reward pool:** none dedicated — Freeport's platform-fee share and order fees feed **Layer C** and the 2% reserve. Escrow is state, not a reward pool.

### LinkGit — Decentralized Git [BUILT]

Git repos as content-addressed objects in HONE-FS; refs recorded on-chain as a verifiable append-only history. Private repos encrypted to the owner's hide key [SPEC'D `docs/LINKGIT_PROTOCOL.md`]. Reserved accounts `linkgit`, `linkgit-registry`. Served over git smart-HTTP at `/git/<owner>/<repo>` (port 4242).

| Entry | Purpose | Status |
|---|---|---|
| `LinkGitRepoCreate` | Register repo, visibility, owner hide key | [BUILT] |
| `LinkGitRefUpdate` | Record commit hash; triggers GC of unreachable objects | [BUILT] chain.rs:1896 |
| `LinkGitAccessGrant`/`Revoke` | Share/revoke repo symmetric key | [BUILT] |
| `LinkGitPruneProof` | Storage node claims GC reward (Merkle root of pruned CIDs) | [BUILT] |
| `LinkGitStorageExtend` | Pay fee to keep orphaned CIDs | [BUILT] |
| `LinkGitServeHeartbeat` | Log a unique per-epoch fetcher (`SHA256(client_ip‖epoch)`) | [BUILT] tx.rs:643 |
| `LinkGitIssue*` / `LinkGitPr*` | Issue & PR chain-objects (`LinkGitPrMerge` authorship-checked) | [BUILT] tx.rs:610,2636 |

**Reward pools (two Layer-B pools, wired at seal `main.rs:1899-1998`):**
- **`LinkGitServeReward`** — serve pool split proportionally by unique-fetcher count; pays the repo owner wallet.
- **`LinkGitBuildReward`** — build pool split proportionally by ref-update push count.

Both are full Layer-B pools (`share = utilization/Σutilization × activity_budget`), subject to the scarcity `payout_factor`. `LinkGitPruneProof` earns from the storage lane.

### Projects — Collaboration & Bounties [BUILT]

Lightweight on-chain workspaces with two task modes: **partial** (fixed bounty, closed on approval) and **full** (ongoing collaborator). Optionally linked to a LinkGit repo.

| Entry | Effect | Status |
|---|---|---|
| `ProjectCreate` | Create workspace, creator = primary owner | [BUILT] tx.rs:1577 |
| `ProjectTask` | Post task (`partial`/`full`, `bounty`, deadline) | [BUILT] tx.rs:1584 |
| `TaskClaim` | Mark claimant as active worker | [BUILT] |
| `TaskSubmit` | Submit work (PR/commit/CID) | [BUILT] |
| `TaskApprove` | partial→pay bounty; full→add collaborator | [BUILT] tx.rs:1605 |

**Reward pool:** none — bounties are direct creator→worker transfers on approval, funded by the creator, not emission. Entry fees feed the reserve / Layer C.

### WASM Smart Contracts [BUILT]

User-deployed WebAssembly via **wasmtime v23** (`ContractEngine`, `contracts.rs:295`). Consensus model is **replicated-deterministic replay**: `ContractDeploy`/`ContractCall` carry the *full* replay inputs (base64 WASM, canonical JSON args) so every node re-executes identical WASM against identical pre-seal state — same discipline as `Mine`. The contract has no key; balance effects are derived at seal, authorized by the deployer's active key [BUILT `entry.rs:371-415`; SPEC'D `docs/CONTRACTS.md`].

| Aspect | Value | Status |
|---|---|---|
| `ContractDeploy` | deterministic `contract_id = derive_contract_address(deployer,epoch,nonce)` | [BUILT] tx.rs:666 |
| `ContractCall` | method + args + `deposit` (debited signer→contract pre-exec) | [BUILT] tx.rs:675 |
| Gas | wasmtime fuel; limit `500_000`/call, 100 ms wall-clock hard abort | [SPEC'D] docs/CONTRACTS.md |
| Host fns | `hone_get/set/delete/balance/transfer/emit/caller` | [SPEC'D] (doc shows legacy `btcpc_*` names) |
| Re-entrancy | `transfer` rate-limited 1/call | [SPEC'D] |
| Immutability | immutable by default; opt-in `upgradeable` via governance timelock | [SPEC'D] |

**Reward pool:** none directly; **contract access/execution fees feed Layer C** and storage-write pricing. Fee is per-entry weight, not per-gas in v1.

### Cross-Chain Bridges — wHONE [BUILT entry layer / SPEC'D contracts]

Lock-and-release bridge between native HONE and wrapped **wHONE** on EVM chains (Ethereum, Base, Arbitrum, Optimism, BSC, Polygon). No burn/mint — locked wHONE stays in the contract pool for the reverse direction [SPEC'D `docs/BRIDGE_TRUST_MODEL.md`].

| Entry | Direction | Effect | Status |
|---|---|---|---|
| `BridgeFund` | custodian in | Mint wHONE up to 4.2M cap against external deposit | [BUILT] tx.rs:1726 |
| `BridgeWrap` | HONE→wHONE | Wrap native HONE | [BUILT] tx.rs:1734 |
| `BridgeUnwrap` | wHONE→HONE | Burn wHONE, FIFO-queue external unlock | [BUILT] tx.rs:1742 |
| `BridgeUnlock` | custodian | Confirm external unlock processed | [BUILT] tx.rs:1750 |

**Trust model [SPEC'D]:** V2 = **3-of-5 multisig** (shindevlin / natoshisakamoto / josh + 2 HSM nodes), EIP-712 (domain `wHONEBridge v2`), daily volume caps, any-1 pause / 3-of-5 unpause. Release (HONE→EVM) needs 3-of-5; lock (EVM→HONE) is trustless event-watching. **V3 [ASPIRATIONAL]:** light-client Patricia-Merkle state-root proofs + 21 staked relayers (slashable). Solidity contracts audit-gated (Zellic/OtterSec/Trail of Bits). Docs still use legacy `wBTCPC`/`btcpc` naming — not yet migrated. **Peer-discovery registries [SPEC'D, code-complete/not-deployed]:** TON (`discovery.rs:TON_REGISTRY_CONTRACT`) and Bitcoin Ordinals fall back to Hive discovery until their constants are set. **Reward pool:** none — bridge ops are custody/multisig-gated value transfers.

### Wiiv — Decentralized Render Platform [SPEC'D / ASPIRATIONAL — DRY-RUN ONLY]

Modality-agnostic (image/video/audio/3D/composite) render marketplace: brief → finished artifact via distributed GPU workers, models, human specialists, storage, reviewers as one supply chain [SPEC'D `docs/WIIV_PROTOCOL.md`]. Reserved accounts `wiiv`/`wiiv-escrow`. Rust job/worker types in `rust/wiiv/`; LLM-producer surface over MCP (`src/mcp/`).

**Not on-chain yet — `grep Wiiv` in `entry.rs` = 0 matches.** No `Wiiv*` ledger entry exists. Planned entries (`WiivWorkerRegister`, `WiivRenderJobPost`, `WiivJobFund`, `WiivRenderBid`, `WiivBidAward`, `WiivMilestoneDeliver/Accept`, `WiivArtifactDeliver`, `WiivJobSettle`, `WiivDisputeOpen/Resolve`, `WiivStorageExtend`) map to a milestone state machine (`pending→active→delivered→accepted`, piecewise tranche release).

**Hard safety boundary [SPEC'D, design intent]:** until wallet-scoped auth + spend caps + live chain routes exist, the Wiiv/MCP layer is **dry-run only** — it plans/quotes/simulates but posts nothing on-chain and moves no value. Subjective quality is never a consensus input.

**Reward pool [ASPIRATIONAL]:** when live, render compute would be a Layer-B activity pool alongside `MineReward`/`RuntimeReward`; worker registration hardware-attested and stake-slashable for capability fraud; escrow via `wiiv-escrow`; disputes fee-funded to reviewers.

### Cross-cutting

- **Entry weights** (`entry.rs:2781+`): commerce writes `STANDARD`; `ProductCreate`/`ContractDeploy`/`ContractCall`/`ProjectCreate`/`ProjectTask` `REGISTRATION`-weight (priced higher to cover permanent state); LinkGit COBs `STANDARD`.
- **Only two app-layer reward pools draw emission directly:** `LinkGitServeReward` + `LinkGitBuildReward` (Layer B). Everything else (commerce, projects, contracts, bridges) is user-funded value transfer + fees flowing to Layer C / reserve — consistent with "the market decides the split," not a static app subsidy.

---

# Part VI — Devices, Clients & The Road Ahead

HONE is "one machine, many roles" — not one process with many responsibilities. Every device runs the roles its hardware can support, and every role that produces work maps to a **reward pool** in the 4-layer model. The market (Layer B per-pool utilization) decides how much each pool earns.

> Naming note: some device docs still say "BTCPC / dreams / 5% clock" in prose (pre-rebrand tail). Current names: **HONE** token, **hunit** base unit, **hone-\*** crates, chain_id **hone**. Static per-pool percentages quoted in older docs are superseded by the dynamic Layer-B model.

### 1. Role Matrix [SPEC'D `docs/ROLE_MATRIX.md`]

Core rules: one machine may run **multiple** roles but **at most one instance** of any given role; each role gets its own process boundary; `hone-all` is a supervisor, not a single runtime loop; the launcher offers only roles the machine can realistically run.

| Role (reward pool / entry) | Phone | Pi / Nebra | Laptop / Desktop | Server | Flipper |
|---|---|---|---|---|---|
| clock — `ClockReward` | Yes | Yes | Yes | Yes | Maybe |
| mine — `MineReward` | Yes, small model | Maybe | Yes | Yes | No |
| storage — `StorageReward` | No by default | Yes | Yes | Yes | No |
| verifier — `VerifierReward` | Maybe | Maybe | Yes | Yes | No |
| reviewer | Maybe | Maybe | Yes | Yes | No |
| sensor — `SensorReward`/`SensorDataCommit` | Yes (GPS+IMU+mic) | Yes | If HW present | If HW present | Yes |
| gateway — `GatewayRewardSplit` | No | Maybe | Maybe | Maybe | No |

**Six** distinct reward-entry types back these seven roles — `reviewer` earns via `InferenceJobPay` escrow (the dispute-review split), not a separate system reward pool; the other six map to live entry variants in `entry.rs` [BUILT]. Notes: **Phone** is a full mobile node (wallet/UI + clock + small miner + sensor fusion), NOT a client-only stub. **Pi/Nebra** = lightweight edge roles. **Laptop/Desktop** = most flexible default (all roles as separate processes). **Flipper** = sensor-first, maybe clock, never a default miner/storage/verifier. Future server-farm miners are an explicit roadmap item, not the default [ASPIRATIONAL].

### 2. Hardware Reference — devices already in the network [SPEC'D]

| Device | Board / MCU | Account | Roles | Earns via |
|---|---|---|---|---|
| **Nebra Indoor** (ex-Helium) | Pi CM3+ / A53 quad, 1GB | shindevlin | clock, storage, gateway | `ClockReward`, `StorageReward`, `GatewayRewardSplit`; onboard temp/load/disk/LoRa sensors → `SensorReward` |
| **Flipper Zero** | STM32WB55, 256KB RAM | josh | sensor (memo-key signing) | Sub-GHz/BLE/NFC/ADC/temp → `SensorDataCommit` |
| **Hyfix MobileCM MCMv3** GNSS base | UM980 triple-band GNSS | natoshisakamoto | sensor (RTK/RTCM3) | `SensorReward` (gnss-base); also cross-network on RTK Direct / GEODNET / onocoy |

Key facts: **Nebra** — SX1302 LoRa concentrator, Semtech UDP 1700, Cayenne LPP, self-heal watchdog if chain stalls >10min, optional CC1101 to receive Flipper Sub-GHz directly. **Flipper** — single-FAP app (active entry point `hone.c` at the crate root; the legacy `hone_wallet.c` is archived under `legacy/`), wallet + background sensor thread (Sub-GHz RSSI, BLE, NFC, GPIO ADC, temp) + USB-CDC JSON, readings buffered to `readings.jsonl`; fw 1.4.3 HAL quirks documented (GCM crashes → XOR for Phase 1). **Hyfix GNSS** — captured via the `rust/hone-gnss-capture/` crate [BUILT] (HTTP-polled ~30s; the standalone `hone-gnss-bridge` binary is archived and the ARP-spoof `gnss-relay` is DEPRECATED). [SPEC'D `docs/hardware/*`]

### 3. Hardware Product Line — three-tier family [SPEC'D `docs/HARDWARE_PRODUCT_LINE.md`]

Every unit is a sensor array + data relay + (Macro) mining node. Two-sided model: owners earn HONE for data; the protocol resells aggregated B2B data and returns **70% of that revenue** to owners — aligning with the `DeviceYieldOptIn` split in code: **70% owner / 20% stakers pro-rata / 10% recycle** [BUILT `entry.rs:1558`].

| Tier | Form / price / BOM | Compute | Uplink | Earns |
|---|---|---|---|---|
| **Micro** "forget it exists" | keychain · ~$80 / ~$35.5 | nRF52840 | BLE 5.3, LoRa SX1262 fallback | Sensor only (`SensorDataCommit`); signs every reading, any relay forwards |
| **General** "always in pocket" | Flipper-like, OLED, open SDK · ~$220 / ~$82.5 | ESP32-S3 + nRF52840 | WiFi/BLE/LoRa/sub-GHz | Sensor (6 categories) + **LoRa gateway relay fees** for nearby Micros (`GatewayRewardSplit`) |
| **Macro** "set and forget" | IP65 rooftop, 18650, 10W solar · ~$500 / ~$182.5 | **Pi CM4** full Linux node | LTE Cat-4 + WiFi6 + LoRa | **Full node**: mining, storage, clock, sensor, gateway simultaneously |

Sensor payload scales by tier (Micro env+IMU+light → General adds CO2/mag/UV/mic/NFC → Macro adds PM/anemometer/rain/spectral/mic-array/optional camera). Data verification (3-layer anti-spoof): secp256k1 device-key signing backed by staked HONE, geographic cross-corroboration, covariance fingerprinting (too-clean synthetic data fails). Named buyers with market evidence: air quality (PurpleAir ~$25M acq.), weather (Tomorrow.io $200M raise), GPS mobility (HERE ~$1B), noise, seismic (USGS/ShakeAlert), light/UV. Go-to-market: DIY kits → retail → B2B city deployments → OEM white-label [ASPIRATIONAL].

### 4. Device Onboarding Roadmap [SPEC'D `docs/DEVICE_ROADMAP.md`]

Phase-ordered by data value; the Pi gateway already supports USB/serial/I2C/GPIO — "plug in and start earning."

| Phase | Device | Data / value | Pool |
|---|---|---|---|
| 0 | **Meshtastic** (T-Beam, Heltec V3, RAK) | millions deployed; LoRa mesh relay via `hone-meshtastic` bridge | gateway / sensor |
| 1 | RTL-SDR + 1090MHz antenna | ADS-B air traffic (passive, high-value) | sensor |
| 2 | BME280 / Pimoroni Enviro | environmental baseline | sensor |
| 3 | PMS5003, MH-Z19B, SGP30 | air quality / pollution | sensor |
| 4 | ADXL345, Grove D7S, Raspberry Shake | vibration / seismic | sensor |
| 5-7 | ultrasonic, soil moisture, INA219, PZEM-004T, ESP32/LoRa/GPS | flood, agriculture, energy, remote nodes | sensor / gateway |

Meshtastic bridge [ASPIRATIONAL] — the prior `btcpc-meshtastic` bridge binary and its setup script are **archived** (under `_archived/`), not in the current `bin/`/`scripts/`. A re-implemented Meshtastic on-ramp (auto-detect `/dev/ttyUSB*`, join a "hone" channel, relay signed packets) is a roadmap item, not live code.

### 5. Clients & Bots [BUILT/partial]

| Client | Location | State |
|---|---|---|
| Desktop (Electron/Tauri) | `clients/btcpc-desktop/` | dir present; rename pending [BUILT-scaffold] |
| Android (Capacitor) | `clients/btcpc-android/`, `clients/hone-android-native/` | miner+sensor+clock Rust BUILT (`android/rust/hone-miner`); www is a stub; needs APK rebuild + JNI wiring [BUILT-but-not-wired] |
| Flipper firmware | `clients/hone-flipper/` | `hone_wallet.c` FAP [BUILT] |
| Telegram bots | `bots/btcpcbot/`, `bots/btcpcwalletbot/` | thin HTTP clients via `/api/bot/*`; rename pending [SPEC'D] |
| Ludicrous (Warp fork) | `ludicrous/` + `plugins/ludicrous/` | [BUILT-scaffold] |
| Relay | `services/btcpc-relay/` (Cloudflare Workers) | live `wss://btcpc-relay.shindevlin.workers.dev/ws` [SPEC'D] |

`flipper_rx.rs` (parse+verify+submit Flipper Sub-GHz/NFC) is BUILT **and JNI-wired** — `NativeFlipperService.nativeIngestFrame` (`android/rust/btcpc-miner/src/lib.rs`) calls `flipper_rx::handle_ble_frame`. The Flipper→chain path exists in native code; do NOT rewrite it in TS. (Remaining gap is client-side surfacing, not the native ingest.)

### 6. Consolidated Roadmap — everything we want to work towards

**Post-launch forward goal** [SPEC'D `docs/POST_LAUNCH_GOAL_AND_COLLABORATION.md`]: *"Make every HONE vertical demonstrably EARN on the live chain, with the node stack non-fragile and the two agents (Beastly=chain, Grouchly=devices) advancing it autonomously."* Roadmap 78/81 done at launch; the only open core roadmap item is **T4-5 HardwareClaim TEE attestation** (Tier 4).

**Six ordered launch milestones** [SPEC'D]:

| # | Milestone | State |
|---|---|---|
| 1 | **Verasens earns** — phone/Flipper/Nebra `SensorDataCommit` → `SensorReward`, verified on-chain | closest to done (sig fix + josh funding unblock) |
| 2 | **Phone is a live earning node** — APK rebuilt, mines qwen2.5-0.5b + sensors, visibly earning | needs APK rebuild |
| 3 | **Inference non-fragile** — embedded candle GGUF default, no 503 on missing daemon | migrating off external Ollama |
| 4 | **Flipper → chain** — `flipper_rx` wired via JNI; sub-GHz/NFC → Verasens | JNI gap |
| 5 | **2nd/3rd verticals live** — freeport, linkgit turned on via the Bullship template | have keys, no live service |
| 6 | **Close T4-5** — hardware TEE attestation, last roadmap item | open |

**Aspirational subsystem targets** (some docs describe the DEPRECATED Node.js path; equivalent work is being redone in Rust):
- **Cross-chain wHONE** — 0.1 HONE credit per HONE earned, claimable as wrapped token on 10 chains (ETH, Base, Arbitrum, Optimism, Solana, TON, Bitcoin, Hive, BSC, Polygon); Solidity `wHONE.sol`/`BridgeLock`/`BridgeReserve` + wrap/unwrap entries [ASPIRATIONAL]. (TON `ton_sign_data` Ed25519 verifier already BUILT.)
- **B2B data marketplace** — verified sensor data resold, 70% to owners; matches BUILT `SensorDataPurchase` + `SensorDataCommit` primitives [ASPIRATIONAL market / BUILT primitives].
- **Container auto-update + role-selection installer** — clock-only mini image, opt-in miner, `HONE_ROLES` env [ASPIRATIONAL].
- **Server-farm miners** — explicitly deferred, never the default [ASPIRATIONAL].
- **SDK / npm package, block explorer upgrade, CI/CD, hardware certification (FCC/CE/IC), Zephyr firmware** [ASPIRATIONAL].
- **Runtime & LinkGit & Tracker verticals** — `RuntimeReward`, `LinkGitServeReward`/`LinkGitBuildReward`, `TrackerCoverageReward` are BUILT entry types awaiting live activation via the Bullship template [BUILT primitives / ASPIRATIONAL activation]. (Note `RuntimeReward` is currently a *dead* system entry — never emitted; activation requires wiring a runtime pool into the seal computation.)
- **Chain-entropy liveness rewards** — `LivenessProof`/`EntropyWitness` code-ready but gated off pending docs + governance vote [SPEC'D].
- **Address module, TOTP 2FA, Private Auth Stack, VRF/VDF consensus (Phase 5/7)** [SPEC'D/ASPIRATIONAL].

**Autonomy gates** [SPEC'D §3.4]: agents build/test/wire/deploy-to-our-nodes freely; only **token transfers, pushing to main, outward-facing/irreversible actions, and secrets** require Shin's explicit approval via a routed sign-request.

**Hardline constraint on all of the above:** a node with zero peers MUST NOT accept or apply any user-submitted entry, and nothing shows as confirmed until sealed in an epoch — every device, every entry type, no exceptions.

---

# Appendix — Master Reward-Pool Table

Every `LedgerEntry` reward pool → resource → layer → status. "System-only" entries are emitted by the seal handler and cannot be user-submitted; they apply immediately on seal, bypassing the pending pool.

| Reward entry | Resource / pool | Layer | Calibration / rate | Status |
|---|---|---|---|---|
| **ClockReward** | Clock (seal infrastructure) | **D** base + also earns via seal | `clock_reward_at` = 0.001 HONE≪era, ×0.5–1.0 uptime, backer-split | [BUILT] |
| **TestnetReward** | Testnet operators | **D** base | `testnet_reward_at` ≈ 0.0005 HONE, from `__testnet_fund__` | [BUILT] |
| Reserve split | recycle / testnet / treasury | **D** (mandatory) | 2% of adjusted_pool → 1.5% / 0.4% / 0.1% | [BUILT] |
| **MineReward** | Inference / mining | **B** | `CALIBRATION_INFERENCE`=10,000; score = tokens×hw×model | [BUILT] |
| **VerifierReward** | Inference verification | **B** | `CALIBRATION_VERIFIER`=5,000; approved value_score | [BUILT] |
| **StorageReward** | Storage (HDD/SSD) | **B** | `CALIBRATION_STORAGE`=10 GB; Merkle-proven bytes × query × tier | [BUILT] |
| **SensorReward** | Sensors / IoT | **B** | `CALIBRATION_SENSOR`=5,000; type-aware `sensor_score` × location | [BUILT] |
| **GatewayRewardSplit** | Gateway (LoRa relay) | **B** (downstream 40% of SensorReward) | `SENSOR_BPS`=6000 / `GATEWAY_BPS`=4000 | [BUILT] |
| **ServiceReward** | Service hosts (containers) | **B** | `CALIBRATION_SERVICE`=24; container-hours | [BUILT] |
| **MempoolReward** | Mempool relay | **B** | `CALIBRATION_MEMPOOL`=10,000; relayed×1000/latency | [BUILT] |
| **TrackerCoverageReward** | BLE tracker / coverage mesh | **B** | `CALIBRATION_TRACKER`=2,000 | [BUILT entry / ASPIRATIONAL activation] |
| **LinkGitServeReward** | LinkGit serve | **B** | `CALIBRATION_LINKGIT`=100; unique fetchers | [BUILT] |
| **LinkGitBuildReward** | LinkGit build/CI | **B** | `CALIBRATION_LINKGIT_BUILD`=10; ref pushes | [BUILT] |
| **RuntimeReward** | Decentralized runtime | *(intended B; not wired)* | `CALIBRATION_RUNTIME`=3 defined; **never emitted** — paid via per-job escrow (host 80% / recycle 20%) instead | [ASPIRATIONAL emission / BUILT escrow] |
| Inference job escrow (`InferenceJobPay`) | Inference marketplace | fee/escrow (feeds **C**) | worker 8000 / verifier 1500 / recycle 500 bps | [BUILT] |
| Runtime job escrow (`RuntimeAttest`) | Runtime marketplace | fee/escrow | host 8000 / recycle 2000 bps | [BUILT] |
| `SensorDataPurchase` | Sensor premium data | fee (feeds **C**) | 80% owner / 20% recycle | [BUILT] |
| `BlobStore` fee | Storage access | fee (feeds **C**) | fee recorded; collection wiring not complete | [SPEC'D/ASPIRATIONAL] |
| Freeport order/platform fee | Commerce | fee (feeds **C** / reserve) | direct value transfer + escrow | [BUILT entries / SPEC'D protocol] |
| Contract call fee | WASM contracts | fee (feeds **C**) | per-entry weight | [BUILT entries] |
| Project bounty | Projects | direct transfer (no pool) | creator→worker on `TaskApprove` | [BUILT] |
| Bridge ops (`BridgeWrap/Unwrap/Fund/Unlock`) | wHONE bridge | none (value transfer) | 3-of-5 multisig gated | [BUILT entries / SPEC'D contracts] |
| `LivenessProof` entropy reward | Chain-entropy liveness | (own entropy pool) | 50% live wallets / 50% recycle | [SPEC'D — gated off] |
| `ModelAnalysisReward` | Model registry | off existing pool (no new issuance) | usage-weighted, capped | [SPEC'D] |
| Wiiv render escrow | Render marketplace | (intended B when live) | dry-run only, no on-chain entry yet | [ASPIRATIONAL] |

**Recycle sink (`__recycle_fund__`):** every leftover path — Layer A damping, sub-100% activity gate, unspent Layer C headroom, reserve split, rounding, scarcity shortfall, slashed stake, rejected/timed-out escrow — credits recycle. In era 5+ recycle pays the network's only reward (`RECYCLE_REWARD_RATE/DENOM = 10/1,000,000` per epoch). **Nothing is ever burned.**
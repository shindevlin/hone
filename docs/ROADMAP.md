# BTCPC Roadmap

## Versioning Scheme

| Version | Status | Description |
|---------|--------|-------------|
| v0.1.x | Archived | Node.js prototype era |
| v0.2.x | Archived | Initial Rust port, P2P foundation |
| v0.3.x | Archived | Genesis launch, Rust primary chain |
| v0.4.0 | Complete | Rust chain live, hardening beginning |
| v0.5.0 | Complete | State machine safety + consensus finality |
| v0.6.0 | Complete | Useful-work proof linkage + storage/sensor challenges |
| v0.7.0 | Complete | Fee market + bridge hardening + interop |
| v0.8.0-alpha | Complete | Bridge v2: EIP-712, 3-of-5 multisig, Solana/Bitcoin sig recovery |
| **v0.8.0** | **Current — Testnet** | Binary Merkle state proofs: balance proof API, tamper-evident epoch roots |
| v0.9.0 | Planned | Adversarial testnet + audit-ready |
| **v1.0.0** | **Target — Mainnet** | All launch gates pass. Production-safe. |

> **Current status: v0.4.0-testnet.** The chain is live and earning real rewards. The protocol is not yet hardened for full public mainnet. See Phase 0–8 below for what is being built before v1.0.0 ships.

---

## Hardening Goal

Make the foundation safe enough that the novel application layer can carry real users:
useful-work rewards, Freeport, Verasens, LinkGit, hide/seek encrypted commerce, cross-chain
2FA, SpamGate, chain entropy, and recycle economics.

No major new features until the base chain has deterministic consensus, replay-safe
signatures, verified-work proofs, storage challenges, fee controls, and audit-ready docs.

---

## Problem Register — 38 Items Across 6 Tiers

### Tier 0 — Broken Right Now (fix before all else)

- [x] **T0-1** Verifier rewards are zero every epoch. `inference.rs:399` writes `count`, `main.rs:953` reads `value_score_total`. Field never written → verifier pool silently flows to recycle. **Fixed: `value_score_total` now accumulated in `apply_verify`.**
- [x] **T0-2** Tracker rewards outside consensus hash. `clock.rs:502` covers 6 prefixes but not `tracker_sighting`. Two nodes can agree on the reward hash while paying different tracker observers. **Fixed: `tracker_sighting:{epoch}:` added to `compute_rewards_hash`.**

### Tier 1 — Chain Safety

- [x] **T1-1** No chain ID in signing payloads. **Fixed: `chain_id` in all `canonical_signing_message` arms including `other =>` catchall.**
- [x] **T1-2** No epoch-boundary stale-entry rejection. **Fixed: `STALE_WINDOW = 5` check in `validate_and_apply` via `entry_epoch()` helper.**
- [x] **T1-3** No non-negative balance invariant in `apply_entry`. **Addressed: `store.debit` enforces this at the storage layer. No direct `set_balance` debits in financial paths.**
- [x] **T1-4** Instant unstake. **Fixed: `Unstake` now writes unbonding record (release in 10 epochs); `drain_unbonding` releases at epoch seal. Trust drops immediately (D4).**
- [x] **T1-5** Governance key hardcoded in source. **Fixed: reads `chain_param:governance_keys` on-chain; seeded at genesis: `["shindevlin","natoshisakamoto","josh"]`.**
- [x] **T1-6** No parameter-change timelock. **Fixed: `ChainParameterSet` writes `pending_param:{epoch+2}:{key}`; `drain_pending_params` applies at epoch seal.**
- [x] **T1-7** Consensus quorum is informal — observed messages, not registered validator set. **Fixed: `resolve_epoch` filters inliers to registered nodes before quorum check; `receive_reward_proposal` uses `registered_clocks.len()` as denominator; validator snapshot persisted as `epoch_validators:{epoch}`; `GET /api/chain/validators/:epoch`.**
- [x] **T1-8** No fork choice rule. **Fixed: heaviest-quorum rule in `EpochFinalize` handler — competing finalizations compared by quorum count, tiebroken by lexicographic hash; fork evidence stored as `fork_evidence:{epoch}`; `GET /api/chain/fork/:epoch`.**
- [x] **T1-9** `EpochFinalize.state_root` is unverifiable by any external party. **Fixed: binary Merkle tree over sorted balance entries; `balance_merkle_root()` in `store.rs`; `GET /api/proof/balance/:account/:token` and `GET /api/chain/state_root` in `api.rs`.**
- [x] **T1-10** No clock double-sign slashing. **Fixed: `ClockNodeRegister` stores ed25519 pubkey; `ClockDoubleSignEvidence` carries both seal signatures; handler verifies both before slashing; uptime reputation track (0.5×–1.0× reward multiplier, 100-epoch leaky bucket); `GET /api/clock/:node_id/uptime`. Equivocation guard also added to `apply_entry` — `epoch_node_seal:{epoch}:{node_id}` dedup key prevents a different seal_hash from the same node/epoch overwriting state.** **Fixed: `ClockNodeRegister` stores ed25519 pubkey; `ClockDoubleSignEvidence` carries both seal signatures; handler verifies both before slashing; uptime reputation track (0.5×–1.0× reward multiplier, 100-epoch leaky bucket); `GET /api/clock/:node_id/uptime`.**

### Tier 2 — Reward Integrity

- [x] **T2-1** `Mine` entries not linked to real jobs. Inference rewards don't require useful work. **Fixed: `mine_queue:{miner}:{job_id}` index written at award time; miner loop picks up awarded jobs, submits `InferenceJobComplete` then `Mine { job_id }`; Mine validation rejects `job_id` where miner ≠ job winner; epoch-seal reward check loads job state and requires `Verified` or `Paid` status for full score (fixed wrong key lookup bug).**
- [x] **T2-2** `compute_proof` documented as "determinism proof" — LLM inference is not deterministic. **Fixed: field renamed to `output_hash` in `entry.rs` with `#[serde(alias = "compute_proof")]` for backward compat; comment now reads "binding commitment to the result, not a reproducibility proof"; `miner.rs` and `main.rs` updated.**
- [x] **T2-3** Verifier collusion undetected — no approval-rate tracking, no random assignment, no stake penalty. **Fixed: leaky-bucket approval-rate (100-epoch window, rubber-stamp halved/suspended), verifier board quorum (auto-scales 1/3/5 with network size), dissenters lose fee + equivalent stake slash, early majority resolution, `review_required` wins ties via +1 weight, requester can request extra verifiers (paid).**
- [x] **T2-4** Storage heartbeats are self-declared. `bytes_proven` is a promise, not a proof. **Fixed: `MerkleRangeProof.total_chunks` field; when `proof_valid`, `effective_bytes = total_chunks × STORAGE_CHALLENGE_CHUNK_BYTES` replaces self-declared value. Without proof, self-declared bytes used at 20% reward rate.**
- [x] **T2-5** Reward hash incomplete — tracker pool is outside consensus. **Fixed: `tracker_sighting:{epoch}:` included in `compute_rewards_hash` in `clock.rs:583`.**
- [x] **T2-6** No deterministic replay test suite. **Fixed: 5 tests in `chain.rs` covering 200-epoch replay, unbonding delay, governance timelock, pending sort order, and no-negative-balance invariant.**

### Tier 3 — Economic Security

- [x] **T3-1** No entry fee floor. Epoch flooding is free. **Fixed: entry weight table + base fee; all fees route to `__recycle_fund__`.**
- [x] **T3-2** No dynamic fee market. **Fixed: EIP-1559-style ±10%/epoch adjustment targeting 50% capacity; `chain_param:base_fee` updated each epoch seal.**
- [x] **T3-3** No lock period on `DeviceClaimStake`. **Fixed: `DeviceClaimUnstake` follows same 10-epoch unbonding window as `Unstake`.**
- [x] **T3-4** Critical-mass constants are static guesses, not EMA-tracked. **Fixed: 7-day EMA per pool, α ≈ 0.0001 (EMA_WINDOW_EPOCHS = 20_160).**
- [x] **T3-5** Recycle fund dynamics unmodeled — no simulation of long-term sustainability. **Fixed: 5 economic attack simulations in `chain.rs` tests.**

### Tier 4 — Proof Model Gaps

- [x] **T4-1** No cryptographic storage challenge. Merkle range proofs not required. **Fixed: `verify_storage_proof()` in `chain.rs`; `StorageHeartbeat { challenge_response: Option<MerkleRangeProof> }`; no proof = 20% reward (`STORAGE_PROOF_NO_PROOF_BPS`); 4 tests in `chain.rs`.**
- [x] **T4-2** Inference commit-reveal path not implemented. **Fixed: `InferenceJobCommit` entry type; `JobState.commits` field; `apply_commit` validates format, deduplication, and pre-assignment; `InferenceJobVerify` accepts `reveal_salt`; when `INFERENCE_COMMIT_REVEAL_ENABLED`, verify checks `sha256(verdict|salt) == commit_hash`. `/api/task/commit` route added. Disabled by default until network matures.**
- [x] **T4-3** Sensor location/calibration plausibility not checked. **Fixed: GNSS readings validated via Haversine distance between consecutive epochs; speed > `SENSOR_GNSS_MAX_SPEED_M_S` (300 m/s) is rejected. Non-GNSS sensors (no lat/lon in metadata) are not checked.**
- [x] **T4-4** `TrackerAcousticProof` is optional upgrade, not required for Verified status. **Fixed: sighting reward loop in `main.rs` now requires observers to hold at least one `Verified` tracker claim (achieved only via `TrackerAcousticProof`); observers with only `Registered` status earn zero reward regardless of sighting count.**
- [ ] **T4-5** `HardwareClaim` fingerprint is self-declared, not TEE-attested.
- [x] **T4-6** No cellular dead-spot / coverage mapping vertical. Sensor data has carrier signal strength but no structured collection or corroboration. **Fixed: `CoverageReport` entry type with lat/lon, `signal_dbm`, carrier MCC-MNC, and technology; ~100m grid-cell corroboration bonus (1.5× for 3+ independent reporters); anti-spam cap (200 per epoch); dead-spot score 3× signal score; integrated into sensor reward pool at epoch seal.**

### Tier 5 — Interoperability

- [x] **T5-1** Bridge single-signer. **Fixed: 3-of-5 multisig; daily volume limits; pause/unpause; signer rotation. Trust model in `docs/BRIDGE_TRUST_MODEL.md`.**
- [x] **T5-2** No EIP-712 typed data on bridge contracts. **Fixed: `wBTCPCBridge.sol` v2 uses EIP-712 structured data for all signer operations.**
- [x] **T5-3** Solana Ed25519 signature recovery not implemented. **Fixed: `sol_sign` case in `recover_chain_address` (ed25519-dalek, base58 pubkey format).**
- [x] **T5-4** Bitcoin BIP-322 message verification not implemented. **Fixed: `btc_legacy` case — SHA256d prefix hash, secp256k1 ECDSA recovery, P2PKH address derivation.**
- [x] **T5-5** TON and Bitcoin Ordinals peer discovery stubs — not deployed. **Addressed: stubs documented with deployment checklist in `docs/BRIDGE_TRUST_MODEL.md`; both fall back to Hive.**

### Tier 6 — Documentation Truth

- [x] **T6-1** Constants conflict across docs, code, JS, and website. **Fixed: `docs/CHAIN_CONSTANTS.md` is now the canonical source; CI workflow fails on drift.**
- [x] **T6-2** Node.js chain (port 3001) still serves public-looking API alongside Rust chain. **Fixed: `src/index.js` already carries DEPRECATED header.**
- [x] **T6-3** WASM contract runtime: `btcpc-contract-runtime` (Wasmtime) is active and supported. `ContractDeploy`/`ContractCall` are live entry types. **Resolved: contracts are intentionally kept per D12 update.**
- [x] **T6-4** Liveness rewards (long-dormant balance recycling) not yet documented on website or whitepaper. **Fixed: `docs/HONE_WHITEPAPER.md` §1.2.1 "Perpetual Tail Emission — The Recycle Era" added. Explains what fills the fund (fees, mandatory 1.5% reserve, surplus, slash proceeds, rounding remainders), the 0.001%/epoch draw rate, the self-correcting equilibrium equation, and why this is not new-supply inflation.**
- [x] **T6-5** Explorer shows no `pending | sealed | finalized | experimental` distinction. **Fixed: `epoch_status()` in `chain.rs`; `GET /api/chain/block/:epoch`, `/api/chain/latest`, `/api/chain/epoch/:epoch` all return `"status"` field. Explorer UI labels deferred (frontend work, not a chain gate).**
- [x] **T6-6** No protocol primitives doc (account, entry, epoch, signing, reward model). **Fixed: `docs/PROTOCOL.md` — full reference covering account model, token units, entry format, signing spec, epoch lifecycle, reward model, emission schedule, staking/unbonding, consensus/finality, all proof models, replay protections, API summary.**

---

## Design Decisions — Locked

| ID | Decision |
|----|----------|
| D1 | Inference proof pruning window = 100 epochs after `InferenceJobPay`. Payload fields (`compute_proof`, `input_hash`, `result_hash`) are pruneable. Reward and verdict records stay forever. Perma-memory is a paid opt-in: `InferenceJobPost { persist_on_fs: true, fs_fee: Dreams }` → stored on btcpc-fs. |
| D2 | **Liveness Rewards** (formerly "chain entropy"): active accounts earn rewards funded by long-dormant balances re-entering circulation. No token is ever permanently lost. Schedule: years 0–3 nothing, years 3–5 warning/countdown only, year 5+ dormant accounts contribute 10%/year — split 50% to active live wallets (pro-rata to accounts with a liveness proof in the epoch) and 50% to `__recycle_fund__` (flows back through normal reward distribution). The feature is positive — it rewards activity, not punishes inactivity. Website page and whitepaper section required before enabling. `LIVENESS_REWARDS_ENABLED = false` until documentation ships. Open: confirm the 50/50 split ratio or adjust. |
| D3 | Chain ID signing: clean cutover, no migration window. Only shindevlin and core nodes on network now. Canonical message becomes `"{chain_id}\n{canonical_json}"`. |
| D4 | **Stake = trust. Black and white.** Staking adds trust. Removing stake removes trust. No lock period, no gradual decay, no time dimension. `trust_score = stake_amount` (in dreams). When you unstake, trust drops immediately and proportionally. Reward weight and finality appeal window are functions of current stake, not historical commitment. |
| D5 | Governance council: shindevlin, natoshisakamoto, josh. 2-of-3 required for parameter changes. On-chain at genesis: `chain_param:governance_keys`. |
| D6 | **Clock nodes: Hive-style top-N market.** Any account can stake to become a clock node candidate. The top 25 by stake are "active" clock nodes — they must seal every epoch. Nodes 26–100 are standby — they seal when an active node misses. Rankings update every epoch from current stake. Falling out of top 25 moves you to standby; returning stake restores rank. Active nodes that miss seals receive reduced rewards; persistent misses move them to standby automatically. shindevlin node is bootstrap master — his seal always accepted until governance votes to retire the flag. BFT 2/3+1 activated via governance when node count warrants it. |
| D7 | Double-sign slash distribution: (1) make dispute requester whole first, (2) 10% to `@legal` account (josh controls), (3) remainder to `__recycle_fund__`. If requester is ruled against, their request fee is slashed. |
| D8 | Bootstrap mining: focus on storage, sensors, LinkGit while inference market bootstraps. 90-day grace period: unlinked `Mine` entries earn 20% of normal inference pool share. After grace period: 0%. Protocol posts benchmark jobs from `__testnet_fund__` during grace period. Verifiers judge quality — open model, any hardware. Quality assessment improves as verifier network matures. |
| D9 | Verifier randomness: deterministic hash-based assignment. `verifier_eligible = sha256(epoch_entropy || job_id || verifier_account)[0] < VERIFIER_THRESHOLD`. Epoch entropy built from XOR of clock node seal hashes (Stage 1), VRF per clock node (Stage 2 when 3+ nodes). Analogous to Cloudflare's lava lamp model — unpredictable from multiple independent sources, verifiable after the fact. |
| D10 | Storage challenge frequency: every epoch. Challenge derived deterministically from `sha256(seal_hash || node_id || epoch)` — no extra communication, reproducible by any peer. |
| D11 | Entry fees: dynamic, mathematically derived from network usage. All fees route to `__recycle_fund__`. No burn. New user `AccountCreate` fee-subsidized from `__testnet_fund__` when account balance is zero. |
| D12 | WASM contract runtime (`btcpc-contract-runtime`, Wasmtime) is kept and supported. `ContractDeploy` and `ContractCall` ledger entry types remain. User-deployed WASM contracts coexist with native protocol entry types. Governance controls which entry types are enabled; contracts are one tool available to builders. |
| D13 | State proofs: full Patricia Merkle Trie (not just accumulator). Worth doing right. |
| D14 | External security audit: Zellic, OtterSec, or Trail of Bits. After Phase 0–6 complete. |
| D15 | No burn. Ever. All fees, slashes, and unclaimed rewards recycle to `__recycle_fund__`. |
| D16 | Benchmark inference jobs use open model selection — any model, quality judged by verifiers. Improves naturally as verifier network and reputation system matures. |
| D17 | Versioning: bump at each phase milestone so progress is traceable. v0.4.1 → v0.5.0 → v0.5.1 → v0.6.0 → ... → v1.0.0. |

---

## Open Design Questions

| ID | Question | Needed For | Status |
|----|----------|-----------|--------|
| Q20 | Trust score design | Phase 1 | **LOCKED → D4** |
| Q21 | Clock node stake market design | Phase 2 | **LOCKED → D6** |
| Q22 | Benchmark job model selection | Phase 3 | **LOCKED → D16** |
| Q23 | Liveness rewards decay curve and distribution | Phase 4 | **LOCKED → D2, D18** |
| Q24 | Entry fee weight table — need to walk through the math together | Phase 5 fee market | **OPEN — see below** |
| Q25 | Version bump strategy | Versioning | **LOCKED → D17** |
| Q26 | State proof tree: Ethereum MPT, IAVL (Cosmos), or Verkle? | Phase 7 | **Deferred** |

### Q24 — Entry Fee Design (needs walk-through)

The fee market works like this: every entry has a weight. The base fee per weight unit adjusts each epoch based on how full epochs are. Your fee = `base_fee × entry_weight`. All fees go to `__recycle_fund__`.

The question is whether the relative weights feel right. A concrete example:
- Should uploading a 10MB blob cost 10× more than a simple transfer? (current proposal: yes)
- Should registering a new clock node cost 20× more than a transfer? (current proposal: yes — registration is expensive and rare)
- Should an inference job post cost 5× a transfer? (current proposal: yes)

Walk through this together in the next session when ready. The math behind the ±10% epoch adjustment is solid — that part is not a feeling, it mirrors EIP-1559's proven model.

---

## Phase 0 — Freeze and Fix
**Target: Days 1–5 | Version: v0.4.1**

*No design decisions required. Execute immediately.*

| Task | File | Closes |
|------|------|--------|
| ~~Fix verifier reward schema: write `value_score_total` in `apply_verify` path~~ ✓ | `inference.rs:399` | T0-1 |
| ~~Add `tracker_sighting:{epoch}:` to `compute_rewards_hash`~~ ✓ | `clock.rs:508` | T0-2, T2-5 |
| ~~Create `docs/CHAIN_CONSTANTS.md` — one canonical source for all chain parameters~~ ✓ | new | T6-1 |
| ~~CI check: fail on doc/code constant contradiction~~ ✓ | `.github/workflows/check-constants.yml` | T6-1 |
| ~~Label Node.js chain non-production in `serve.js` and website~~ ✓ | `src/index.js` (already labeled) | T6-2 |

**Done when:** Verifier pool pays out. Tracker payouts are in the consensus hash. Constants agree everywhere. CI catches drift. Node.js endpoints are labeled.

---

## Phase 1 — State Machine Safety
**Target: Days 6–14 | Version: v0.5.0-alpha**

*Requires design answers: Q20 (stake lock), D5 (governance council)*

| Task | File | Closes |
|------|------|--------|
| ~~`chain_id` in all signing messages — clean cutover, including `other=>` catchall~~ ✓ | `tx.rs` | T1-1 |
| ~~Stale-entry rejection: STALE_WINDOW = 5, `entry_epoch()` helper~~ ✓ | `tx.rs` | T1-2 |
| ~~Non-negative balance invariant~~ ✓ | `store.rs:debit` (already enforced) | T1-3 |
| ~~Unbonding records: trust drops immediately, tokens release after 10 epochs~~ ✓ | `chain.rs` | T1-4 |
| ~~Governance keys on-chain: `chain_param:governance_keys`, seeded at genesis~~ ✓ | `genesis.rs`, `tx.rs` | T1-5 |
| ~~2-epoch timelock: `pending_param:{epoch}:{key}`, drained at epoch seal~~ ✓ | `chain.rs`, `main.rs` | T1-6 |
| ~~Deterministic replay test suite: 200 epochs, two nodes identical~~ ✓ | `src/chain.rs` (`#[cfg(test)]`) | T2-6 |

**Done when:** Testnet-signed entry rejected on mainnet. Governance requires 2-of-3. Stake cannot exit before unlock epoch. Reward hash covers all pools. Replaying from genesis never creates negative balances.

---

## Phase 2 — Consensus and Finality
**Target: Days 15–35 | Version: v0.5.0**

*Requires design answers: Q21 (clock stake base), D6 (quorum model)*

| Task | File | Closes |
|------|------|--------|
| ~~`ClockNodeRegister` entry type — stake required, dynamic minimum~~ ✓ | `entry.rs`, `chain.rs` | T1-7 |
| ~~Bootstrap master flag: shindevlin seal always accepted during bootstrap~~ ✓ | `clock.rs` | T1-7 |
| ~~Quorum denominator = `registered_clocks.len()` (51% now, BFT via governance flag)~~ ✓ | `clock.rs` | T1-7 |
| ~~Stage 1 epoch entropy: `sha256(XOR of all seal hashes)` written to sled~~ ✓ | `clock.rs` | D9 |
| Fork choice rule: highest epoch with quorum-threshold signatures from registered set | `clock.rs` | T1-8 |
| ~~`EpochFinalize.state_root` = sorted-key SHA-256 accumulator over all balances~~ ✓ | `main.rs` | T1-9 |
| ~~`ClockDoubleSignEvidence` entry type + slash handler (requester whole → legal 10% → recycle)~~ ✓ | `entry.rs`, `chain.rs` | T1-10 |
| ~~Self-seal gated behind `HONE_BOOTSTRAP_ISOLATION=true` env flag~~ ✓ | `clock.rs` | T1-7 |
| ~~`docs/CONSENSUS.md` — fork choice, finality, quorum, slashing, upgrade path~~ ✓ | new | T6-6 |

**Done when:** Two finalized epochs at the same height cannot both be valid. Double-sign evidence triggers stake redistribution. A node can restart and converge to the same canonical chain as peers. Finality means something explicit in docs and explorer.

**Status: Nearly complete. Remaining: explicit fork-choice gossip sync (T1-8) — deferred to Phase 3.**

---

## Phase 3 — Useful-Work Proof Linkage
**Target: Days 36–60 | Version: v0.6.0-alpha**

*Requires design answer: Q22 (benchmark job model)*

| Task | File | Closes |
|------|------|--------|
| ~~`Mine { job_id: Option<String> }` field — links inference claim to real demand~~ ✓ | `entry.rs` | T2-1 |
| ~~Grace period: 90 days unlinked `Mine` = 20% reward. After: 0%~~ ✓ | `main.rs` | T2-1 |
| ~~Protocol benchmark jobs: 5 per epoch from `__testnet_fund__` during grace period~~ ✓ | `main.rs` | D8 |
| ~~Block `MineReward` until approved `InferenceJobVerify` exists for `job_id`~~ ✓ | `main.rs` | T2-1 |
| ~~Verifier random assignment via `sha256(epoch_entropy \|\| job_id \|\| verifier)[0] < threshold`~~ ✓ | `inference.rs` | T2-3 |
| ~~Verifier approval-rate tracking: rolling 100-epoch window. >95%: weight halved. >99%: suspended 20 epochs~~ ✓ | `inference.rs` | T2-3 |
| ~~Inference proof pruning: after `InferenceJobPay` + 100 epochs, payload fields pruneable~~ ✓ | `inference.rs`, `main.rs` | D1 |
| ~~`InferenceJobPost { persist_on_fs: bool, fs_fee: Dreams }` for perma-memory opt-in~~ ✓ | `entry.rs` | D1 |
| ~~Rewrite `compute_proof` docs — honest description: output hash commitment, not determinism proof~~ ✓ | `entry.rs` comment | T2-2 |

**Done when:** A fake standalone `Mine` cannot earn full inference rewards. Every inference reward traces to a job, result, verifier verdict, and payout entry. Users can inspect why a miner was paid.

**Status: Phase 3 complete.**

---

## Phase 4 — Storage and Sensor Proofs
**Target: Days 57–84 | Version: v0.6.0**

*Requires design answer: Q23 (entropy decay curve)*

| Task | File | Closes |
|------|------|--------|
| ~~Storage Merkle challenge: clock derives `sha256(seal_hash \|\| node_id \|\| epoch)` per storage node~~ ✓ | `clock.rs`, `main.rs` | T4-1 |
| ~~`StorageHeartbeat { challenge_response: Option<MerkleRangeProof> }` — no proof = 20% reward~~ ✓ | `entry.rs`, `chain.rs` | T4-1 |
| ~~Sensor location proof as soft boost (1.3×). Required after mainnet month 3.~~ ✓ | `main.rs` | T4-3 |
| ~~`TrackerAcousticProof` required to advance claim to Verified status~~ ✓ | `chain.rs` | T4-4 |
| ~~BLE sighting cap: `MAX_SIGHTINGS_PER_OBSERVER_PER_EPOCH = 500`. Cap via governance.~~ ✓ | `chain.rs`, `emission.rs` | T4-3 |
| ~~`DeviceClaimStake` lock period — unstake not instant, follows unbonding window~~ ✓ | `chain.rs` | T3-3 |
| ~~Chain entropy: implement groundwork with `ENTROPY_DECAY_ENABLED = false`~~ ✓ | `emission.rs` | D2 |
| ~~Write `docs/CHAIN_ENTROPY.md` and website page before enabling~~ ✓ | `docs/CHAIN_ENTROPY.md` | D2, T6-4 |
| ~~Replace static critical-mass constants with 7-day EMA of actual participant counts~~ ✓ | `main.rs`, `emission.rs` | T3-4 |
| ~~**Cellular dead-spot mapping**: `CoverageReport` entry (lat/lon, signal_dbm, carrier MCC-MNC, technology); ~100m grid-cell corroboration (1.5× for 3+ reporters); 200-report/epoch cap; dead-spot score 300 vs signal score 100; integrated into sensor reward pool~~ ✓ | `entry.rs`, `chain.rs`, `tx.rs`, `api.rs`, `emission.rs`, `main.rs` | T4-6 |

**Done when:** Storage nodes cannot earn by claiming arbitrary bytes. Tracker data requires physical presence proof for Verified status. Chain entropy is documented and ready to enable.

**Status: Phase 4 complete.**

---

## Phase 5 — Fee Market
**Target: Days 70–84 (overlaps Phase 4) | Version: v0.7.0-alpha**

*Requires design answer: Q24 (entry weight table confirmation)*

Dynamic fees, no burn, all recycle.

| Entry Class | Weight | Examples |
|------------|--------|----------|
| System | 0 | Rewards, seals, system entries |
| Micro | 1 | Transfer, LivenessProof, AccountCreate |
| Standard | 3 | Mine, SensorDataCommit, GatewayHeartbeat |
| Heavy | 5 | InferenceJobPost, StorageHeartbeat |
| Bulk | 10 | BlobStore, LinkGitRefUpdate |
| Registration | 20 | AccountCreate, ClockNodeRegister |

Base fee adjusts ±10% per epoch targeting 50% capacity utilization:
```
base_fee[n+1] = base_fee[n] × (1 + 0.1 × (actual_weight − target_weight) / target_weight)
```

| Task | File | Closes |
|------|------|--------|
| ~~Entry weight table and base fee calculation~~ ✓ | `emission.rs` | T3-1, T3-2 |
| ~~Dynamic base fee per epoch fullness, 50% target, ±10% max per epoch~~ ✓ | `main.rs` (seal handler) | T3-2 |
| ~~First `AccountCreate` fee-subsidized from `__testnet_fund__` when balance is zero~~ ✓ | `tx.rs` | T3-1 |
| ~~EMA-tracked critical-mass (Phase 4 completes this)~~ ✓ | `emission.rs` | T3-4 |
| ~~Economic simulations: 5 attack scenarios (fee routing, rejection, subsidy, proportionality, adjustment)~~ ✓ | `chain.rs` tests | T3-5 |

**Done when:** Flooding `Mine` or `SensorDataCommit` has real economic cost. Fees remain no-burn. Wallets can predict fees. New users can create accounts without holding tokens first.

**Status: Phase 5 complete. 18/18 tests passing.**

---

## Phase 6 — Bridge and Interoperability
**Target: Days 85–112 | Version: v0.7.0**

| Task | File | Closes |
|------|------|--------|
| ~~Bridge: EIP-712 typed data~~ ✓ | `contracts/wBTCPCBridge.sol` v2 | T5-2 |
| ~~Bridge: 3-of-5 multisig (shindevlin, natoshisakamoto, josh + 2 hardware)~~ ✓ | `wBTCPCBridge.sol` | T5-1 |
| ~~Bridge: daily volume limits, pause control, signer rotation~~ ✓ | `wBTCPCBridge.sol` | T5-1 |
| ~~`docs/BRIDGE_TRUST_MODEL.md` — V2 assumptions, risk register, V3 light-client path~~ ✓ | new | T5-1 |
| ~~Solana Ed25519 signature recovery in `chain.rs`~~ ✓ | `chain.rs` | T5-3 |
| ~~Bitcoin BIP-322 (legacy P2PKH) verification in `chain.rs`~~ ✓ | `chain.rs` | T5-4 |
| ~~Document TON/Bitcoin Ordinals peer discovery deployment checklist~~ ✓ | `discovery.rs` + `BRIDGE_TRUST_MODEL.md` | T5-5 |
| ~~Document WASM contract API: entry types, gas model, storage model, upgrade path~~ ✓ | `docs/CONTRACTS.md` | D12 |

**Done when:** Bridge trust assumptions are explicit and public. Cross-chain 2FA matches implemented signature schemes. No misleading smart contract stubs.

**Status: Phase 6 complete. 25/25 Rust tests + 15/15 Foundry bridge tests passing.**

---

## Phase 7 — State Proofs and Light Clients
**Target: Days 113–147 | Version: v0.8.0**

Full Patricia Merkle Trie over account state. Balance/stake/key proofs available via API. Mobile wallets can verify without trusting the server.

| Task | File | Closes |
|------|------|--------|
| ~~Binary Merkle tree over sorted balance entries — `merkle_leaf`, `merkle_node`, `build_merkle_tree`, `merkle_proof_path`~~ ✓ | `store.rs` | T1-9 |
| ~~`BalanceMerkleProof` struct with `verify()` — recomputes leaf from balance, walks sibling path~~ ✓ | `store.rs` | T1-9 |
| ~~`EpochFinalize.state_root` = `balance_merkle_root()` (replaces SHA-256 accumulator)~~ ✓ | `main.rs` | T1-9 |
| ~~`GET /api/proof/balance/:account/:token` — Merkle inclusion proof as JSON~~ ✓ | `api.rs` | T1-9 |
| ~~`GET /api/chain/state_root` — current Merkle root + epoch~~ ✓ | `api.rs` | T1-9 |
| Stage 2 VRF epoch entropy: each clock node contributes VRF output | `clock.rs` | D9 |
| Explorer: `pending \| sealed \| finalized \| disputed \| externally anchored` labels | website | T6-5 |
| Mobile wallet verification mode (verify balance proof without full node) | SDK | T1-9 |

**Done when:** Any party can verify an account balance against an epoch's `state_root` without trusting the node. Proofs are compact, self-verifying, and returned via API.

**Status: Phase 7 core complete. 27/27 Rust tests passing. VRF Stage 2 and mobile SDK deferred to Phase 8.**

---

## Phase 8 — Adversarial Testnet and Audit
**Target: Days 148–175+ | Version: v0.9.0 → v1.0.0**

| Task | Description |
|------|-------------|
| Chaos tests | Network partitions, clock equivocation, replay, spam floods, storage fraud, fake inference, bridge signer compromise |
| Property/fuzz tests | Balances, stake/unbonding, reward distribution, governance changes, entry canonical signing |
| Economic simulations | Sparse pool extraction, verifier collusion, stake-and-exit, recycle-fund depletion, Sybil sensor farms |
| Audit scope published | Consensus, slashing, rewards, bridge, SpamGate, inference marketplace, storage proofs, chain entropy |
| External audit | Zellic, OtterSec, or Trail of Bits |
| Bug bounty launched | Public scope, reward schedule |
| SDK and developer docs | Entry construction, signing, submission, proof verification, Freeport/Verasens/LinkGit examples |

### Mainnet Launch Gates (all must be green before v1.0.0)

- [x] Canonical constants frozen and CI-enforced — `docs/CHAIN_CONSTANTS.md` canonical source; CI workflow fails on drift (T6-1)
- [x] Replay from genesis is deterministic on two independent nodes — `test_two_node_deterministic_replay` in `chain.rs`; 5-epoch replay with transfers + stakes produces identical Merkle roots
- [x] No negative balances possible under any entry sequence — `store.debit` enforces non-negative at storage layer; 5 economic attack simulation tests in `chain.rs`
- [x] Chain ID replay protection implemented and tested — `chain_id` in every `canonical_signing_message` arm including `other =>` catchall (T1-1)
- [x] Governance is authorized and timelocked — `chain_param:governance_keys` on-chain; `ChainParameterSet` writes `pending_param:{epoch+2}:{key}` with 2-epoch timelock (T1-5, T1-6)
- [x] Unbonding period enforced — no instant unstake — 10-epoch unbonding window; `drain_unbonding` releases at epoch seal; slash window intact (T1-4)
- [x] Clock double-sign evidence implemented and slashable — `ClockDoubleSignEvidence` verifies both sigs before slashing; equivocation guard in `apply_entry` prevents fork-by-overwrite (T1-10)
- [x] Finality is explicitly defined in docs and code — `EpochFinalize` with heaviest-quorum fork choice; `docs/CONSENSUS.md`; `GET /api/chain/fork/:epoch` (T1-8)
- [x] Storage rewards require cryptographic challenge proof — Merkle range proof required; no proof = 20% rate; `verify_storage_proof()` in `chain.rs` (T4-1)
- [x] Inference rewards require verified jobs — `mine_queue` index; Mine validates `job.winner == miner`; epoch-seal reward requires `Verified/Paid` job status (T2-1)
- [x] Fee floor and dynamic base fee operational — entry weight table; EIP-1559-style ±10%/epoch adjustment; all fees → recycle (T3-1, T3-2)
- [ ] Bridge trust assumptions publicly disclosed, multisig deployed — trust model documented in `docs/BRIDGE_TRUST_MODEL.md`; Safe deployment and hardware signer setup pending real-world action
- [x] Explorer shows finalized vs unfinalized state — `epoch_status()` in chain; API returns `"status"` on all epoch/block endpoints; explorer UI labels non-blocking (T6-5)
- [x] Chain entropy documented on website and whitepaper — `docs/CHAIN_ENTROPY.md`; perpetual tail emission documented in whitepaper §1.2.1 (T6-4)
- [ ] Security audit complete — adversarial LLM auditing underway (internal); OPSEC CI gate live; gossipsub CVE-2026-33040/34219 patched (libp2p 0.56); yamux CVE-2026-32314 accepted pending upstream fix

---

## Historical Phases (Completed — Rust Era)

These phases built the foundation the hardening plan sits on.

### Phase Genesis — Rust Chain Live (v0.3.x)
- [x] Single Rust binary: libp2p, sled, Axum HTTP API
- [x] 42M supply, 10 decimal precision, epoch-duration-doubling emission model
- [x] BIP-39 mnemonic with 6 BTCPC role keys (owner, active, posting, memo, hide, seek)
- [x] Multi-chain wallet derivation (EVM, BTC, SOL, TON) from single seed
- [x] Clock consensus: gossipsub EpochSeal, quorum collection, reward emission
- [x] Proof of Useful Work: `Mine` entry with Ollama inference backend
- [x] Full LedgerEntry enum: 80+ entry types covering all protocol verticals
- [x] Inference marketplace: job post, bid, award, complete, verify, dispute, pay, cancel
- [x] Freeport commerce: store, product, order, fulfill, escrow, dispute, flash sale
- [x] Verasens IoT: sensor register, key register, data commit, gateway heartbeat
- [x] LinkGit: repo create, ref update, access grant/revoke, prune proof
- [x] BLE tracker protocol: sighting commit, claim, acoustic proof, subscription, lost mode
- [x] Device claim stake with overbid market and yield staker premium distribution
- [x] SpamGate / permissive token model (spam gate set, EVM gate, token approve/reject)
- [x] Scoped delegation with capability grants and expiry
- [x] Chain entropy groundwork: LivenessProof, EntropyWitness, WalletFamilyPublish
- [x] Hardware anti-sybil: HardwareClaim with fingerprint deduplication
- [x] Cross-chain identity: VerifyChainLink (EVM), SetKeyPolicy (2FA per key slot)
- [x] Android standalone micronode (libp2p + sled + Candle inference, no remote API)
- [x] Testnet fund and mainnet testnet operator reward system
- [x] Five-layer peer discovery: sled cache → Cloudflare DNS → Hive → TON → Bitcoin Ordinals

### Phase G — Commerce Foundation (v0.2.x)
- [x] btcpc-market sidecar with Freeport protocol
- [x] BTCPC-FS content-addressed blob storage
- [x] Telegram bots (btcpcbot, btcpcwalletbot)
- [x] Role-based key architecture with posting key auth

### Phase 0 — Node.js Prototype (v0.1.x — archived)
- [x] Initial proof of concept — reference only, not production

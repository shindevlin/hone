# BTCPC Stabilization & Launch-Readiness PRD (Fable 5, independent review)

> **Status:** Governing plan as of 2026-07. Supersedes the *priority ordering*
> of `docs/PLATFORM_PRD.md` — it does NOT delete that document. The
> Verasens/Freeport/LinkGit product thinking there is worth keeping as a
> **parking lot** for post-mainnet work. The sequencing below is what the next
> 3–6 months must actually be, given the project's real state: a testnet chain
> moving real rewards, pre-audit, with a currently-unbuildable Rust toolchain
> and an in-flight security fix (PR #7) that was committed without ever being
> compiled.
>
> **Governing principle:** order by *blast radius to real money*, not by product
> earning potential. A feature that can silently mispay or fork the chain
> outranks a feature that could earn revenue. Nothing that moves fiat, extends
> credit, or adds a new trust boundary ships before the base chain is audited.

---

## Part 1 — Independent assessment: has recent work been on the right track?

### What has been genuinely good (agree)

1. **Consensus/reward hardening (ROADMAP Tiers 0–5) is real, correct, well-sequenced.**
   The T0 fixes (verifier rewards silently flowing to recycle because
   `inference.rs` wrote `count` and `main.rs` read `value_score_total`; tracker
   rewards outside the consensus hash) are the highest-priority class of bug on
   a live chain — silent divergence and silent mispayment. Fixing those first
   was correct.

2. **The theft-of-funds vulnerability was the highest-value recent output.**
   `tx.rs` had `SensorReading`/`SensorRegister`/`GatewayHeartbeat` in the
   allowlisted pass-through arm with zero signature verification, and `main.rs`
   pays `SensorReward`/`GatewayRewardSplit` to whatever account those
   unauthenticated entries name. Genuine "submit a reading naming any account,
   that account gets paid" hole. Worth more than every product vertical
   combined.

3. **The GitHub-wide marketing-truth review was worth doing.** Grepping
   `INNOVATIONS.md` claims against the actual Rust and finding "Lucid Pruning,"
   "Genesis Dreams," "Sparse Merkle Tree ~1KB proofs," and "resource-aware
   auto-throttle" have **no code** is exactly the honesty a pre-launch project
   needs.

4. **The 100× dreams-constant discrepancy is a real bug surfaced.** Code is
   canonical (`DREAMS_PER_BTCPC = 10_000_000_000`, `lib.rs:12`); CLAUDE.md and
   the settlement design say `100,000,000`. That it *reappears inside the new
   PRD's own settlement math* is itself evidence the PRD was written faster
   than checked.

### Where the plan went wrong (disagree)

1. **`PLATFORM_PRD.md` phase ordering optimizes the wrong axis — product
   ambition, not risk-to-money.** Its order defers the **reputation layer to
   Phase 6**, while Phases 1, 3, and 7 all explicitly depend on it. You cannot
   build the thing that depends on reputation five phases before reputation
   exists. The correct axis for a pre-mainnet chain with live money is *blast
   radius*: "what breaks the chain or steals funds" → "what makes the base
   usable" → "what grows the product." By that axis, most of `PLATFORM_PRD.md`
   is Phase 4+ work.

2. **Ambition is wildly mismatched to stated maturity.** ROADMAP says testnet,
   external audit "Planned (Phase 8, not started)," bridge multisig not
   deployed. Against that, `PLATFORM_PRD.md` proposes institutional B2B
   dashboards with **Stripe USD billing**, a **USD→BTCPC settlement bridge**
   moving real fiat, **micro-lending against future earnings**, an **agent
   payment economy with unlinkability**, **proof-of-location-as-a-service**, a
   **seller insurance pool**, and Freeport as "a real Amazon competitor." That
   is 5–7 net-new product surfaces, several venture-scale on their own, several
   moving fiat or extending credit — on top of a chain that has not been
   audited and cannot currently be built. Building a fiat billing bridge and a
   lending desk before an external auditor has seen the consensus layer is
   putting the storefront on an un-inspected foundation.

3. **The "stop, escalate" pivot was the best recent decision, not a failure of
   nerve.** When asked to implement the sensor reward split and finding it
   required changing the reward-timing model, escalating rather than pushing
   through was correct. On a live financial chain that reflex is exactly right.
   Reinforce it.

4. **The daily parallel-agent workflow is the riskiest idea in the plan and
   must not run against chain code.** Concrete reasons rooted in this repo:
   (a) You cannot verify output — a documented rustc ICE makes
   `cargo check`/`test` impossible right now; PR #7 (the *security fix*) was
   committed without ever being compiled. An automated fan-out that can't
   compile what it writes generates unfalsifiable "done" claims. (b) Parallel
   writes to one chain's invariants don't compose — two locally-correct reward/
   validation edits can jointly produce a divergence bug neither introduced
   alone. (c) The PRD already shows drift from unsupervised generation (the
   100×-wrong constant reappearing after being flagged elsewhere in the same
   doc). **Policy: parallel workflow allowed for design docs / marketing-truth
   prose only. Any change under `rust/`, or to reward/validation/consensus/fee
   logic, requires human review + a green CI build. Never tick a Rust item
   "done" that didn't compile and test.**

5. **Busywork-vs-fixes imbalance in recent history.** 4 of the last 5 commits
   were `docs:` adding ~1,320 lines of PRD prose; the one code commit (the
   security fix) is untestable right now. A pre-mainnet chain's log should be
   dominated by hardening + tests, not roadmap documents for products that are
   phases away.

### One concrete defect the broken build hid (in PR #7)

`SensorReading` now requires a signature via `check_signature(... "posting")`,
but it had **no arm in `canonical_signing_message`** — it fell through to the
`other =>` catchall (`tx.rs`), which signs the full entry JSON *including the
server-set `epoch`*. The function's own doc comment warns clients sign a
message that must NOT include server-set fields, because a client can't know
the sealing epoch in advance. As written, the "correctly-signed reading
applies" test could never pass. **Fixed in commit `9e393c65`** (dedicated
`SensorReading`/`SensorRegister`/`GatewayHeartbeat` signing arms that exclude
`epoch`) — but that fix, too, is unverified until the toolchain is repaired.
**Do not merge PR #7 until the build runs and this is tested for real.**

---

## Part 2 — The PRD

### Phase 0 — Unblock the ability to verify anything (Days 1–7) — HIGHEST PRIORITY

Nothing else is real until Rust code can be compiled and tested. Every "done"
claim on a Rust item since the ICE commit is unverifiable.

- [x] **0.1 Restore a working build/test toolchain. DONE** (option **b** +
  **a**): feature-gated `matrix-sdk` off by default (the sole ≥1.93 forcer),
  and pinned rustc 1.90 via checked-in `rust/rust-toolchain.toml` (commit
  `abd2ec4f`). Core `btcpc-node` compiles clean and `cargo test --bin
  btcpc-node` runs to completion (254 passed / 0 failed). The rustc ICE only
  manifests at ≥1.93; 1.90 avoids it and matrix is no longer forcing ≥1.93.
  Original analysis below kept for the record.
  Resolve the rustc ICE
  (query stack: `check_mod_deathness`/`resolver_for_lowering` on the `api`
  module; reproduces on unmodified `main` at rustc 1.93 and 1.95; 1.90 avoids
  it but fails the `matrix-sdk` ≥1.93 dependency floor). Options in priority
  order: **(a)** pin a rustc that satisfies the matrix-sdk floor AND avoids the
  ICE, in a checked-in `rust-toolchain.toml`; **(b)** feature-gate `matrix-sdk`
  — it is a transport-cascade dependency (Matrix alt-transport), used in exactly
  one file (`src/matrix_transport.rs`, wired at `main.rs:99,1117`), not
  consensus-critical — so `btcpc-node` core builds without it and off a lower
  rustc; **(c)** last resort, split the workspace so `crates/btcpc-types` + core
  chain crates build on a stable toolchain independent of the transport crates.
  **Acceptance:** `cargo test -p btcpc-node` runs to completion from a clean
  checkout, producing a pass/fail count.
- [x] **0.2 CI gate: no Rust merge without a green build. DONE** (`abd2ec4f`):
  `.github/workflows/test.yml` `rust-check` job pins 1.90 and runs `cargo test
  --bin btcpc-node` on every PR touching `rust/`. Verified green on the PR #8
  merge (both `unit-tests` and `rust-check` passed). The structural fix for
  "PR #7 was committed untested."
- [x] **0.3 Reconcile the dreams constant everywhere. DONE.** Fixed the 6
  factual occurrences of the wrong `1 BTCPC = 100,000,000 dreams` →
  `10,000,000,000` (canonical `DREAMS_PER_BTCPC`): `CLAUDE.md`,
  `docs/MINER_GUIDE.md`, `docs/PLATFORM_PRD.md` (settlement seam),
  `marketing/FAQ.md`, `marketing/GLOSSARY.md`. Confirmed NO code used the wrong
  value (money math was already safe — this was docs only). Acceptance met:
  `grep -r "= 100,000,000 dreams"` returns only the notes that *describe* the
  historical discrepancy, no factual claims.

### Phase 1 — Land the security fix, correctly and verified (Days 7–14)

- [x] **1.1 Fix and merge PR #7. DONE — merged via PR #8** (`82c3f5ef`). Signing
  arms (`9e393c65`) verified under the now-working build: 7 sensor-auth tests
  pass, including "correctly-signed reading for a keyed owner applies" (the case
  the broken catchall previously made impossible). PR #7 was rebased into PR #8
  and closed as merged-via-#8.
- [x] **1.2 Audit for sibling holes. DONE — PR #8 + the LinkGit follow-up
  commit.** Audited every pass-through entry. Found and fixed:
  - **7 exploitable** (money/escrow): `TrackerFoundConfirm` (critical —
    bounty theft), `TrackerLostMode`, `TrackerClaim`, `TrackerSubscription`
    (== claimer), `DeviceYieldUnstake` (== staker), `SensorDataCommit`,
    `StorageHeartbeat` (reward-driving, signature required). Bound `signed_by`
    to the authorized account + verify. Tests prove forge-rejection.
  - **10 impersonation** (NEEDS-SIG): commerce metadata + device/sensor key
    registration — signature now required.
  - **~13 LinkGit repo-control** entries (RefUpdate, Access*, Pr*, Issue*,
    RepoCreate, PruneProof, StorageExtend): bound `signed_by` to the entry's
    actor field + verify. Closes third-party forgery of repo actions.
  - **Inert** (orders/escrow via btcpc-market sidecar; sighting/routing
    gossip) left in pass-through, documented.
  - Full suite 251/0 on 1.90.
- [ ] **1.2-followup-B — LinkGitServeHeartbeat reward-farming (schema change,
  separate PR).** `LinkGitServeHeartbeat` has **NO `signed_by` field** and its
  `owner`-attributed serve counts drive the `LinkGitServeReward` pool split at
  epoch seal (`main.rs`, `linkgit:serve_count:{epoch}:{repo_id}`). An attacker
  floods heartbeats for their OWN repo with varied `requester_hash` values
  (bypassing per-epoch dedup), inflating their serve_count and capturing a
  disproportionate share of the LinkGit reward pool — dilutes every honest
  repo's reward. Same class as the original SensorReading bug, and same fix
  shape: add `signed_by` to the struct (schema change → breaking, needs the
  bootstrap-skip compat path + client update), verify it, and bind it to the
  serving node. Scoped as its own branch/PR because it's a breaking schema
  change, not a signature-only fix. **Acceptance:** forged heartbeats for an
  account with a posting key are rejected; a test proves an attacker cannot
  inflate serve_count for reward.
- [ ] **1.2-followup-C — LinkGit repo-ACL authorization.** The Phase 1.2
  LinkGit fix binds `signed_by == self-declared actor`, which stops anonymous/
  third-party forgery but does NOT verify the actor is *authorized on the repo*
  (e.g. `PrMerge`/`AccessGrant` should require repo-owner or granted-
  collaborator status, checked against repo ACL state). Needs repo-state lookup
  at validation time. **Acceptance:** a non-collaborator's signed PrMerge on
  someone else's repo is rejected.
- [ ] **1.3 Client-side signing for the one live producer.**
  `btcpc-android/src/sensors.rs` submits unsigned and has no BTCPC posting-key
  infra (only a libp2p transport identity). Add a device posting-key signing
  path reproducing 1.1's signing message. **Acceptance:** the Android client
  produces readings that verify.

### Phase 2 — Prove the chain is deterministic and safe under adversity (Days 14–45)

ROADMAP Phase 8 pulled forward — it is the actual gate to mainnet.

- [ ] **2.1 Adversarial/chaos test suite:** partitions, clock equivocation,
  replay, spam floods, storage-proof fraud, fake inference, double-sign. Each
  attack has a test that demonstrates the defense or files a bug.
- [ ] **2.2 Deterministic replay across two nodes, in CI**, extended to every
  reward-bearing entry type (sensor, storage, tracker, inference, coverage) —
  the paths where silent divergence has already bitten twice. **Acceptance:**
  CI asserts identical Merkle roots across a multi-entry-type replay.
- [ ] **2.3 Close the two open mainnet gates.** Bridge multisig deployed with
  real signers OR explicitly disabled and documented as disabled (not
  half-live). Audit scope frozen and published.
- [ ] **2.4 Engage the external audit** (Zellic / OtterSec / Trail of Bits).
  Scope: consensus, slashing, rewards, the sensor/storage/tracker payout paths
  (highest historical bug density), bridge, fee market. Internal "adversarial
  LLM auditing" is explicitly NOT counted as this.

### Phase 3 — Make the base usable and honest (Days 30–75, overlaps Phase 2)

- [ ] **3.1 Marketing truth pass** (safe on the parallel workflow — it's prose):
  each oversold `INNOVATIONS.md` claim is built or deleted.
- [ ] **3.2 Bot reliability + tests.** Bots are the live user surface. Tests
  for balance/transfer/create (before/after parity), then split monoliths, then
  self-heal audit. Verifiable, doesn't touch consensus.
- [ ] **3.3 Explorer finality labels.** Chain already returns `status`;
  explorer should visibly distinguish pending/sealed/finalized.

### Phase 4 — Reputation layer (Days 60–100) — built in the RIGHT order

Invert `PLATFORM_PRD.md`'s ordering (it made this Phase 6 while three earlier
phases depend on it). Reputation is a chain primitive and the shared
dependency; it must exist before anything that gates on trust.

- [ ] **4.1 Adversarial-first design doc** (sybil, wash-trading, collusive
  vouching, slow-burn-then-abuse), each with a specific chain-level defense.
- [ ] **4.2 Implement as one account-keyed primitive** from existing on-chain
  signals (mining history, `InferenceReviewVote`, `OrderDispute`, sensor
  purchase/complaint, `RuntimeSlash`) + the per-`sensor_id` projection the
  sensor-fairness overlay needs. Deterministic fixed-point (basis points, no
  floats in consensus state); a test per gaming pattern.

### Phase 5 — First product vertical, end to end, native token only (Days 90–150)

Take **one** vertical all the way, in BTCPC only — no fiat, no USD bridge, no
lending. Recommend **Verasens sensor economics + aggregation** (most chain
groundwork; security work already centered there).

- [ ] **5.1 Implement the creation-fee/usage-split model** (design already in
  `PLATFORM_PRD.md` 1.1) now that Phase 4 reputation exists to gate it — with
  the reputation multipliers wired to the real primitive, not a stub. The four
  deferred tests (spam nets negative; high-query out-earns low-query; slow-burn
  decays; unknown sensor fails closed) run and pass.
- [ ] **5.2 Verasens aggregation service** ingesting only signed/verified
  readings. 3+ sensor types aggregate correctly; restart doesn't lose/
  double-count; unverified readings excluded from paid answers.
- [ ] **5.3 EXPLICITLY DEFERRED to post-mainnet:** the institutional USD
  dashboard, Stripe billing, USD→BTCPC settlement bridge. Moving fiat on top of
  an unaudited chain is the highest-risk item in the entire existing plan and
  has no business shipping pre-audit. Revisit only after Phase 2's audit is
  clean.

### Post-mainnet parking lot (explicitly NOT the next 6 months)

Freeport-as-Amazon, LinkGit-as-GitHub, the agent economy with payment
unlinkability, proof-of-location-as-a-service, seller insurance pool, and
**especially micro-lending against future earnings** (undercollateralized
credit is the single most gameable mechanism proposed and belongs nowhere near
a pre-audit chain). Real ideas; v2 ideas. Keep the design thinking in
`PLATFORM_PRD.md`; don't build them until the base is audited and one vertical
has proven the pattern.

---

## Bottom line

The chain-hardening work was on the right track. The security find was
excellent and the escalate-don't-push-through reflex was correct — reinforce
both. The strategic drift into a multi-vertical, fiat-billing, lending-desk
roadmap is premature by roughly a full mainnet cycle. The single most important
thing to fix first isn't in either existing PRD: **you currently cannot build
or test the chain, and an untested security fix shipped because of it.** Fix
the toolchain, gate merges on a green build, verify PR #7 for real, pull the
audit forward, build reputation before the things that depend on it, and prove
exactly one vertical in native token before wiring up a single dollar.

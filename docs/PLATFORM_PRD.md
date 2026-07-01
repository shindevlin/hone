# BTCPC Platform PRD — Verticals, Storage, and New Products

> **Canonical, all-up backlog for BTCPC's product verticals**, distinct from
> `docs/ROADMAP.md` (protocol/consensus hardening phases) and
> `docs/SELF_HEAL_PRD.md` (fail-path auto-recovery). This file is where
> Verasens, unified storage, Freeport, LinkGit, bots, and new verticals get
> built out — with real code, real tests, on every part, every day.
>
> Ground truth as of 2026-07: the chain core (`rust/btcpc-node`) is solid —
> ~41k LOC, 226 tests, all entry types below already exist in
> `crates/btcpc-types/src/entry.rs`. The verticals themselves (the products
> built ON TOP of those entry types) are thin-to-nonexistent. This PRD closes
> that gap, in priority order.

---

## Ground rules for every phase below

1. **Build AND test.** No item is done when the code compiles — it's done
   when there's a real test proving the behavior, same standard as
   `rust/btcpc-node`'s existing 226 tests.
2. **Every phase ends with something runnable**, not just a design doc.
   Design-only items exist (marked explicitly) but must produce a decision
   record other phases can build against — not open-ended musing.
3. **Adversarial by default.** Every new economic mechanism (reputation,
   insurance pool, lending) must include a "how would someone game this"
   section before it's considered designed, not after launch.
4. **Self-heal parity.** Anything long-running (phone light-agents, sensor
   ingest, dashboards) follows the same rule as `docs/SELF_HEAL_PRD.md`:
   disconnection/failure is normal, not exceptional, and state must recover
   without a human running a command.
5. **Feature branches, real commits, no AI attribution** — same as
   everywhere else in this repo (see root `CLAUDE.md`).

---

## Priority order (why this order)

Verasens first — user-confirmed as the strongest earning vertical for the
protocol, and it already has the most chain-level groundwork
(`SensorReading`, `SensorReward`, `SensorDataPurchase`, `GatewayRewardSplit`
all exist). Storage unification second, because Verasens, LinkGit, and
compute-as-a-whole all depend on it. Freeport third (biggest scope, most
undesigned). LinkGit fourth (weakest existing code, needs the storage layer
from Phase 2 first). Bots fifth (smaller, more mechanical). New verticals
last, gated on the foundation being real.

---

## Phase 1 — Verasens: aggregation + intelligence layer

Verasens is NOT "sensor readings on chain" — that already exists
(`SensorReading`). Verasens is what turns raw readings into something
*queryable and intelligent*, across every sensor type: Helium miner sensors,
Flipper Zero (custom firmware, full sensor suite — not just wallet), GNSS
(currently offline, hardware reconnects when this phase is ready for it), and
every future sensor type without a code change per sensor.

### 1.1 — Economic model: two-part sensor payment

- [ ] **Design doc: creation-fee vs. usage-payment split.** `SensorReward`
  currently pays the reporter directly and fully at creation (see
  `chain.rs` `LedgerEntry::SensorReward` handling — straight credit, no
  usage gating). `SensorDataPurchase` already exists with a real fee split
  (owner majority + storage contract rate + recycle) for buyer-initiated
  purchases. The gap: **creation should mint a SMALL fee only** (covers
  cost of producing/storing the blob), with the *bulk* of sensor-owner
  earnings coming from `SensorDataPurchase` events over the data's
  lifetime. Write the exact split percentages and the reasoning, informed
  by real numbers (avg blob size, avg query rate expected per sensor type).
- [ ] **Fairness-to-seller design (reputation-gated).** A sensor owner who
  reports garbage/spam data must not out-earn one reporting high-quality,
  frequently-queried data under a flat creation fee. Tie the creation fee
  and/or purchase-split percentage to a **sensor reputation score** (ties
  into Phase 6 reputation layer — do not build two separate reputation
  systems). Adversarial check: what stops someone spamming thousands of
  low-effort readings to farm creation fees? (Likely answer: creation fee
  must be below cost-to-produce for a rational spammer, real usage payment
  must dominate lifetime earnings — prove this with numbers, not intuition.)
- [ ] **Implement the split in `chain.rs`** — adjust `SensorReward`
  application to the small-fee model; confirm `SensorDataPurchase`'s
  existing split logic matches the design. Add tests: spam-reporting
  scenario should be unprofitable; legitimate high-query sensor should
  out-earn a low-query one over N epochs.

### 1.2 — Universal sensor ingest (any sensor type, no new chain code per type)

- [ ] **Audit current sensor entry coverage** — `SensorReading` (generic),
  `CoverageReport` (cellular dead-spot specific). Confirm the generic
  `SensorReading.metadata: Option<serde_json::Value>` field is sufficient
  for arbitrary sensor payloads (GNSS position+accuracy, sub-GHz signal
  capture, Flipper's IR/NFC/RFID/BLE reads, Helium miner witness data) or
  whether a typed schema-per-sensor-class is needed. Decide and document.
- [ ] **Build the Verasens aggregation service** (new: likely a Rust sidecar
  alongside `btcpc-market`/`linkgit`, e.g. `rust/verasens/`, replacing the
  empty root `verasens/` README stub). Ingests `SensorReading` events from
  the chain, groups by sensor type + geography + time window, and exposes a
  query API. This is the actual product — not a passthrough of raw chain
  reads.
- [ ] **Tests**: ingest correctness (readings from 3+ sensor types
  aggregate independently and correctly), query correctness, and a
  self-heal test (aggregation service restart does not lose or double-count
  in-flight readings).

### 1.3 — Flipper Zero custom firmware (full sensor suite)

- [ ] **Inventory Flipper Dolphin's onboard sensors/radios** actually worth
  exposing (sub-GHz, 125kHz RFID, NFC, infrared, BLE, GPIO) and which map to
  a Verasens-useful reading type.
- [ ] **Extend/replace `clients/btcpc-flipper`** (currently ~240 lines, C,
  prototype only) to capture and submit `SensorReading` entries for each
  supported sensor, signed by the device key already described in
  `DeviceKeyRegister`/`DeviceClaimStake`.
- [ ] **Test on real hardware** — no faked emulator success. A submitted
  reading must be independently verifiable on-chain and correctly ingested
  by the Phase 1.2 aggregation service.

### 1.4 — Institutional dashboard (B2B, USD billing)

- [ ] **Design the institutional product**: companies log in, browse/query
  aggregated Verasens intelligence, pay in **USD** (not BTCPC) — needs a
  fiat billing integration (Stripe or equivalent), auth (proper
  login/session, not the existing bot-JWT pattern which is consumer-scale),
  and a rate plan (per-query, subscription, or both — decide and document
  in this file before building).
- [ ] **Build the dashboard** (new web app, e.g. `verasens/dashboard/` or
  a service alongside `website/`) — login, query builder against the
  aggregation service's API, USD invoicing/billing, usage history.
- [ ] **USD → BTCPC settlement bridge** — when a company pays USD, the
  underlying `SensorDataPurchase` fee still needs to be posted on-chain in
  BTCPC/dreams so sensor owners get paid in the native token regardless of
  how the buyer paid. Design and implement this conversion step explicitly;
  do not let USD payments bypass on-chain settlement.
- [ ] **Tests**: end-to-end — company account created, query made, USD
  charged, on-chain `SensorDataPurchase` posted with correct amount, sensor
  owner credited.

---

## Phase 2 — Unified storage (chain data + sensor blobs + git objects + versioning)

One storage system, not four bolted-on ones. Everything the chain does is
compute and should be compensated the same way (per user directive) —
storage nodes included.

- [ ] **Audit current storage primitives** — `BlobStore`, `StorageHeartbeat`,
  `HiveReplicaCommit`/`HiveReplicaVerify`, `StorageReward`. Confirm these
  already form a general-purpose content-addressed store usable by chain
  snapshots, Verasens sensor blobs, and LinkGit objects alike, or identify
  what's missing for a genuinely unified layer (likely: per-consumer
  namespacing/quotas, git-specific object typing).
- [ ] **Design doc: "everything is compute" reward unification.** User
  directive: GPU inference, sensor capture, AND storage should all be
  compensated as facets of one "proof of useful work" model, not three
  separate reward pools with different fairness properties. Decide whether
  this means literally merging reward pools/weights, or keeping separate
  pools but under one shared fairness framework (e.g. one reputation/stake
  input feeding all of them). Document the decision and reasoning.
- [ ] **Git object storage on the unified layer** — LinkGit (Phase 4) needs
  git blobs/trees/commits stored via this same system, content-addressed,
  with the existing `LinkGitPruneProof`/`LinkGitStorageExtend` entries
  governing retention. Confirm/implement this wiring.
- [ ] **Git versioning + team collaboration primitives** — real multi-
  contributor workflows: branches, merges, permissions, PRs (entry types
  `LinkGitPrCreate`/`LinkGitPrComment` already exist — confirm they're
  actually wired to a working merge/review flow, not just recorded events).
  This is what makes LinkGit a *competitor*, not just encrypted blob
  storage with git-shaped metadata.
- [ ] **Tests**: a storage node serving sensor blobs, chain snapshots, and
  git objects simultaneously, with correct reward attribution per data type
  and a chaos test (node drops mid-serve, self-heals, no data loss beyond
  acceptable replica-count degradation).

---

## Phase 3 — Freeport: real Amazon competitor

Not peer-to-peer escrow trades — a real marketplace where sellers can run
independent stores AND curate cross-store listings into a specialized,
niche storefront (drop-ship model), reconciling multi-warehouse shipping.

- [ ] **Design doc: cross-store curation mechanic.** How does Seller B's
  storefront "pull" Seller A's product listing into their own store?
  Options to evaluate: (a) affiliate/referral model — B's store links to
  A's actual listing, B earns a cut, A fulfills; (b) true drop-ship — B's
  store IS the checkout, order routes to A for fulfillment, B never touches
  inventory. User's framing ("pull items... to make their store like a
  drop shipping store") points to (b). Decide and document which, and why.
- [ ] **Design doc: multi-warehouse shipping reconciliation.** A buyer's
  cart may contain items from 3 different sellers' warehouses. Design (not
  yet decided whether on-chain or off-chain per user — that's an open
  question to resolve IN this design pass): shipping cost calculation,
  split-shipment handling, delivery time estimates across origins, and how
  this is presented to the buyer as one coherent checkout. Whatever is
  decided, existing entries `OrderPlace`/`OrderFulfill`/`OrderCancel`/
  `OrderDispute`/`EscrowRelease` must still be the settlement backbone —
  don't invent parallel settlement.
- [ ] **Implement cross-store listing/curation** in `btcpc-market` (or
  wherever Phase 3.1's design lands it).
- [ ] **Implement shipping reconciliation** per the Phase 3.2 design.
- [ ] **Seller insurance pool** (new vertical, folds in here since it's
  Freeport-specific): sellers stake BTCPC into a shared pool; buyers get
  automatic refund from the pool on verified non-delivery/dispute
  (`OrderDispute` already exists as the trigger entry). Adversarial design
  required: what stops a seller intentionally under-fulfilling to drain the
  pool, or a buyer falsely disputing to trigger payout? Answer: gate payout
  on **reputation** (ties to Phase 6) — sellers with strong reputation get
  cheaper insurance premiums; buyers with a history of false disputes lose
  standing to trigger auto-payout and fall back to manual arbitration.
  Design this explicitly, adversarially, before writing code.
- [ ] **Tests**: cross-store checkout with items from 2+ sellers/
  warehouses settles correctly on-chain; insurance payout triggers
  correctly on a verified non-delivery and is blocked/flagged on a
  suspicious dispute pattern.

---

## Phase 4 — LinkGit: real git-hosting competitor

Currently 414 lines total — a skeleton. Depends on Phase 2's unified
storage + versioning work landing first.

- [ ] **Build real repo CRUD + object storage on the Phase 2 unified
  layer** — this is the baseline "actually works" bar: clone, push, pull,
  branch, merge round-trip against `rust/linkgit`, backed by real storage,
  not stubbed.
- [ ] **Build real PR/issue workflow** on top of the existing
  `LinkGitPrCreate`/`LinkGitPrComment`/`LinkGitIssueCreate`/
  `LinkGitIssueComment`/`LinkGitIssueClose`/`LinkGitIssueReopen` entries —
  confirm these drive an actual review/merge UI or CLI flow, not just
  chain-recorded events nobody reads.
- [ ] **CI/build marketplace** (new vertical, folds in here): nodes run a
  repo's build/test pipeline for a fee, using the same "prove real work"
  compute-mining infrastructure as inference mining (`LinkGitBuildReward`
  entry already exists in `chain.rs` reward pools — confirm/implement the
  actual build-execution + proof-of-completion flow behind it). This is
  what makes LinkGit "the place teams actually work," not just storage.
- [ ] **Tests**: full clone→branch→PR→merge cycle against a live LinkGit
  repo; a submitted build job runs, proves completion, and pays out
  correctly; a fraudulent build-completion claim is caught/slashed
  (`LinkGitBuildReward` should have or need a challenge path — check and
  add if missing, mirroring `RuntimeChallenge`/`RuntimeSlash`).

---

## Phase 5 — Bots: fixed and usable

Smaller scope, but currently the least reliable user-facing surface next to
the client stubs.

- [ ] **Add real tests** for `bots/btcpcbot` and `bots/btcpcwalletbot` —
  currently untested single-file monoliths (29k/90k LOC). Start with the
  highest-traffic commands (balance, transfer, create) before full
  coverage.
- [ ] **Split the monoliths** into testable modules — command handlers,
  API client, formatting — without changing behavior (verify via the new
  tests from the previous item, before/after parity).
- [ ] **Self-heal audit** — apply the same rule as
  `docs/SELF_HEAL_PRD.md`: bot process crashes, API timeouts, Telegram API
  rate limits must all auto-recover, never silently go dark.
- [ ] **Decide fate relative to OpenClaw wallet plugin** (see
  `docs/OPENCLAW_COMPAT_PRD.md` Phase 2) — once that plugin exists, does it
  replace these bots for OpenClaw users, run alongside, or should these
  bots become the reference implementation the OpenClaw plugin wraps?
  Document the decision here once Phase 2 of the OpenClaw PRD is reached.

---

## Phase 6 — Cross-cutting: reputation layer (build once, use everywhere)

Referenced by Phase 1 (sensor fairness), Phase 3 (insurance pool), and
needed by Phase 7 (agent economy). Build ONE system; every vertical reads
from it rather than inventing its own trust score.

- [ ] **Design doc, adversarial-first.** Enumerate every known reputation-
  gaming pattern before designing the mechanism: sybil accounts, wash
  trading/wash-querying, collusive vouching, slow-burn trust-then-abuse
  (build reputation honestly, then cash it out with one large fraud). For
  each, write the specific chain-level defense (stake-weighting, time-decay,
  slashable vouches, rate limits tied to account age/stake).
- [ ] **Implement as a shared primitive** — likely a reputation score keyed
  by account, computed from existing signals already on-chain (mining
  history, `InferenceReviewVote` outcomes, `OrderDispute` history, sensor
  data purchase/complaint history, `RuntimeSlash` history) rather than a
  new independently-gamed input.
- [ ] **Wire into Phase 1 (sensor fairness) and Phase 3 (insurance pool)**
  as the first two consumers, proving the shared design actually serves
  both before adding more consumers.
- [ ] **Tests**: each gaming pattern from the design doc has a
  corresponding test proving it's caught or economically irrational.

---

## Phase 7 — New verticals (build after Phases 1–6 land; do not front-run the foundation)

All approved by user. Each gets its own design-then-build treatment; do not
start code until the design item is done and reviewed.

- [ ] **7.1 — General compute marketplace beyond inference.** Extend
  "prove real work, get paid" to rendering/data-processing/scientific
  compute. `ScientificResult`/related entries already referenced in
  `chain.rs` weight tables — audit what exists vs. what's stubbed, then
  build out a real job-post/bid/award/verify/pay cycle mirroring the
  inference job flow (`InferenceJobPost` → `...Pay`) exactly, so the
  pattern is proven and just needs a new job-type, not new mechanics.
- [ ] **7.2 — Agent economy marketplace (OpenClaw-linked).** Agents hire
  other agents for sub-tasks, settled in BTCPC. **Must be secure and
  private by design** — default to no visibility into what agents are
  working on or who's paying whom outside the transacting parties. Design
  the privacy mechanism explicitly (encrypted job payloads? Payment
  unlinkability?) before building. Cross-reference
  `docs/OPENCLAW_COMPAT_PRD.md` Phase 4 (agent-to-agent settlement) — this
  IS that phase; keep the two docs in sync, don't fork the design.
- [ ] **7.3 — Proof-of-location / anti-spoof verification service.**
  Paid third-party API: "was this device physically here at this time,"
  backed by signed Verasens sensor readings (ties directly to Phase 1).
  Explicit use case: feeds Freeport's multi-warehouse shipping problem
  (verified pickup/delivery location proofs). Depends on Phase 1 landing
  first (needs real aggregated sensor data to verify against).
- [ ] **7.4 — Resilient phone light-agent / light-storage nodes.** No
  existing repo found for this (checked GitHub — 8 repos under
  shindevlin, none are a dedicated phone light-agent app); this is new
  build work, not an audit of existing code. Core requirement, stated by
  user: **"if it is gone, it should come back"** — phones disconnect
  constantly (battery, backgrounding, connectivity loss) and this must be
  treated as the NORMAL case, not a failure. Design session-state
  persistence and reconnect/resync behavior BEFORE writing the mining/
  storage-contribution logic — the resilience model is the hard part, not
  the phone-side feature work. Closest existing footholds: `website/app.html`
  (already usable from a phone browser per README) and
  `clients/btcpc-android` (currently a thin Capacitor scaffold). Follow the
  same self-heal rule as `docs/SELF_HEAL_PRD.md`.
- [ ] **7.5 — Micro-lending against provable earnings.** Freeport sellers,
  storage operators, and miners have on-chain earning history. Design
  undercollateralized lending against future earnings, auto-repaid from
  the reward stream. Depends on Phase 6 (reputation) landing first — this
  is exactly the kind of mechanism that needs a trust score to not be
  immediately gamed by borrow-and-vanish.

---

## How agents/sessions work this PRD

1. Pick the highest-priority unticked item, working phases IN ORDER — do
   not start Phase 3 items while Phase 1 items remain unticked, etc. Phase
   6 and 7 items may pull forward only when their explicit phase
   dependency (noted per-item) is satisfied early.
2. Build real code + real tests for every implementation item. Design items
   must produce a written decision in this file, not just be discussed and
   forgotten.
3. Work on a feature branch (`platform/<phase>-<short-slug>`), run the
   relevant test suite, commit with a real descriptive message (no AI
   attribution per root `CLAUDE.md`), push, and leave merge-to-main for
   review — do not merge your own work.
4. Tick the box here, note the branch/commit/PR, and if the item is a
   design doc, paste the actual decision inline (not just "done, see
   branch") so this file stays a readable source of truth on its own.
5. If an item reveals the plan is wrong (e.g. Phase 2's audit finds the
   storage primitives don't generalize the way assumed), STOP, do not force
   a bad fit, rewrite the affected item(s) with what was actually learned,
   and flag it clearly for the next session/user review.

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

## GitHub-wide review findings (2026-07)

A full review of every repo under `shindevlin` on GitHub (8 repos total:
`btcpc`, `btcpc-desktop`, `btcpc-gnss-capture`, `btcpc-market`,
`btcpc-marketing`, `phonehome`, `btcpc-terminal`, `btcpc-p2p`) turned up two
things worth acting on:

1. **`btcpc-marketing`'s `INNOVATIONS.md` oversells several features as
   built when they are not.** Verified by grep against `rust/btcpc-node`:
   - **Lucid Pruning** (chain self-compression via AI inference) — **no
     code found.** Pure marketing copy.
   - **Genesis Dreams / on-chain inscriptions** — **no code found.**
   - **Sparse Merkle Tree state proofs** ("~1KB proof for any balance,
     fixed-size regardless of account count") — **no SMT implementation
     found.** What DOES exist: `state_root`, `merkle_root_transactions`,
     `merkle_root_compute_proofs` as plain 32-byte fields in the block
     header (`crates/btcpc-types/src/block.rs`) — a normal merkle-root
     design, not a Sparse Merkle Tree, and no proof-generation code was
     found to back the "~1KB proof" claim.
   - **Resource-aware mining / auto-throttle** ("detects when user is at
     the computer, reduces intensity") — **no code found.**
   - **Finality blocks every 100 epochs for instant sync** — **partially
     real, oversold.** A real `snapshot_replication.rs` module exists with
     tests, but it's a per-account snapshot mechanism, not the described
     "every 100 epochs, full network state snapshot, new nodes sync in
     seconds" feature.
   - **What IS real and correctly described:** Proof of Useful Work
     (accurate — this is the actual chain model), Decentralized Clock
     Nodes (real, `ClockReward`/`ClockNodeRegister` exist and work), Hive-
     style 4-key account model (real, matches `AccountCreate`/key-role
     structure seen throughout `entry.rs`), Cross-chain mining rewards
     (real — `cross_chain_finality.rs` exists with a live API route),
     Consensus-based epoch finalization (real — matches
     `EpochSeal`/`EpochFinalize` and the finalization-consensus work done
     this session), Mempool with tx hashes (real, matches existing P2P
     mempool code reviewed in this session).
   - **Action item:** `marketing/INNOVATIONS.md` (mirrored 1:1 locally from
     `btcpc-marketing`) needs a truth pass — either build the missing
     features for real, or stop claiming them. This is now a tracked item,
     see Phase 8 below. Do not let unverified marketing claims keep
     circulating as if they're shipped.

2. **`phonehome` (github.com/shindevlin/phonehome) is a working, real
   precedent for Phase 7.4's "resilient phone light-agent" problem** —
   correcting the earlier note in Phase 7.4 that said "no existing repo
   found for this." `phonehome` is a Rust CLI + Telegram bridge that
   already solves the *reconnection* half of the problem: a
   `poll_interval_ms`-based Telegram poller, session-resume via a
   `session_file`, and a systemd `Restart=always` pattern — i.e. "if it's
   gone, it comes back" is already a proven, working pattern in this
   exact author's other project, just for a CLI-bridge use case, not
   mining/storage. Phase 7.4 has been updated to reference this as a
   starting pattern rather than building resilience from zero.

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

- [x] **Design doc: creation-fee vs. usage-payment split.** Done — see decision
  block inline below. `SensorReward`
  currently pays the reporter directly and fully at creation (see
  `chain.rs` `LedgerEntry::SensorReward` handling — straight credit, no
  usage gating). `SensorDataPurchase` already exists with a real fee split
  (owner majority + storage contract rate + recycle) for buyer-initiated
  purchases. The gap: **creation should mint a SMALL fee only** (covers
  cost of producing/storing the blob), with the *bulk* of sensor-owner
  earnings coming from `SensorDataPurchase` events over the data's
  lifetime. Write the exact split percentages and the reasoning, informed
  by real numbers (avg blob size, avg query rate expected per sensor type).

  ---

  #### DECISION (design, 2026-07) — creation-fee vs. usage-payment split

  **Status:** design only. No `chain.rs` change is made by this item; that is
  the separate implementation item "Implement the split in `chain.rs`" below.
  This is a doc-only item, so there are no tests to run — stated explicitly
  and intentionally. The reputation-gating overlay is NOT designed here (it is
  the parallel "Fairness-to-seller" item below); this decision only marks the
  two places reputation will later multiply into.

  ##### Ground-truth read (verified in code, not assumed)

  - **Units.** `crates/btcpc-types/src/lib.rs:12` defines
    `DREAMS_PER_BTCPC = 10_000_000_000` (1e10). **This contradicts the
    "1 BTCPC = 100,000,000 dreams" figure used in some loose docs** (including
    the 1.4 settlement-seam record above) — the **code is canonical:
    1 BTCPC = 10,000,000,000 dreams**. All numbers below use the code value.
    (`SUPPLY_CAP_DREAMS = 42_000_000 * 1e10`, `emission.rs:107`, confirms it.)
  - **`SensorReward` today** (`chain.rs:1936`): a straight
    `credit(node_id, NATIVE_TOKEN, *amount)` with no usage gating. `amount` is
    not a fixed per-reading fee — it is this owner's *slice of the Layer-B
    sensor reward pool* for the epoch, computed in `main.rs` (~1632–1661) from
    `sensor_score(reading_count, sensor_type)` (`emission.rs:273`) times the
    1.3× location boost (`SENSOR_LOCATION_BOOST_BPS = 13_000`,
    `emission.rs:575`), grouped by owner. So "creation reward" today is already
    *inflationary pool emission*, not a fee paid by any buyer.
  - **`SensorDataPurchase` today** (`chain.rs:1981`): the entry doc comment
    (`entry.rs:1032`) and the inline comment (`chain.rs:1982`) both say
    **"80% owner / 15% storage contract / 5% recycle"**, but the **actual code
    implements only two shares**: `owner_share = fee * 80/100` and
    `recycle_share = fee - owner_share` (i.e. **80% owner / 20% recycle**).
    **There is no storage-contract share wired at all** — the 15%/5% split is
    fictional in the current binary. This is the code-vs-comment discrepancy
    the item asked to flag. **Resolved below** (option B).

  ##### (1) Creation fee — exact amount/formula per `SensorReading`

  **Decision: creation mints a flat, tiny, cost-recovery fee of
  `SENSOR_CREATION_FEE_DREAMS = 2_000` dreams per accepted `SensorReading`
  (= 0.0000002 BTCPC), NOT the score-weighted pool slice it mints today.** The
  score-weighted `SensorReward` pool emission is retained ONLY as the wrapper
  the fee is paid through (existing `SensorReward` entry plumbing and per-owner
  grouping stay intact); the *magnitude* drops to the flat fee. Score still
  decides *relative* ordering for any residual pool weighting but no longer
  sets the absolute earning — earning comes from usage (section 2).

  Formula (per reading, before the reputation multiplier the fairness item
  will add):

  ```
  creation_fee_dreams(reading) = SENSOR_CREATION_FEE_DREAMS   // = 2_000, flat
  ```

  **Why 2,000 dreams and why flat.** The fee must *recover the marginal cost
  of producing and storing the blob and no more* — deliberately below the
  real-world cost of manufacturing a fake reading, so spamming readings to
  farm creation fees is strictly unprofitable (the adversarial requirement the
  fairness item states in numbers). Real blob sizes, per representative sensor
  type on chain:

  | Sensor type (`sensor_type`) | Example | Avg blob / reading | Readings/epoch (30s) typical |
  |---|---|---|---|
  | `continuous` | temp / RH | ~64–128 B | 1–3 |
  | `sampled` | GNSS fix | ~256–512 B | ~30 (1 Hz) |
  | `event` | Flipper NFC/IR read | ~1–4 KB | 0–5 |

  Take the *largest* common blob (~4 KB `event`) as the sizing case. Storage
  is compensated separately via the Layer-B `StorageReward` pool at the
  network's proven byte rate; the marginal chain cost of *accepting and
  indexing* one reading (a `data_hash` + metadata record, not the blob body,
  which lives in `BlobStore`) is a few hundred bytes of state. With the base
  fee floor `BASE_FEE_MIN_DREAMS = 1_000` (`emission.rs:530`) being the
  smallest fee the chain charges for one weight unit, **2,000 dreams
  (2 × floor)** covers accept+index+one replica-epoch of the hash record with
  margin, and is ~4–6 orders of magnitude below the electricity+hardware+
  bandwidth cost a rational spammer pays to fabricate and sign a reading. Flat
  (not score-weighted) so producing *more* junk never scales creation income
  super-linearly — spammer fee income is at best linear in readings while cost
  is also linear, and the fee is below cost, so the line never crosses into
  profit. **Reputation plugs in here** as a multiplier
  `creation_fee_dreams × rep_mult` (fairness item owns `rep_mult`); nothing
  about the flat base needs to change for that.

  ##### (2) `SensorDataPurchase` split — exact percentages

  **Decision (reconciling the discrepancy — option B, "make the comment
  true"): 80% owner / 15% storage-contract / 5% recycle.** The intended
  three-way split in the comment is the correct economic design (storage nodes
  keeping the blob queryable must be paid out of each purchase, not only out of
  the separate `StorageReward` emission pool); the current two-way code is the
  bug, not the intent. Exact integer math the implementation item must use
  (floor on the first two, remainder to recycle so nothing is lost to
  rounding):

  ```
  owner_share   = fee * 80 / 100
  storage_share = fee * 15 / 100
  recycle_share = fee - owner_share - storage_share   // ~5%, absorbs remainder
  ```

  `storage_share` is credited to the storage-contract account currently
  serving the batch (same account family `StorageReward` pays); the
  implementation item resolves exactly which account id that is. **Reputation
  plugs in here** as a shift of the *owner* percentage band (high-rep owner
  keeps closer to 80%, a low-rep owner's owner-share is reduced and the
  difference routed to recycle) — the fairness item owns the exact curve; this
  decision only fixes the baseline 80/15/5.

  ##### (3) Numeric model — usage dominates creation over the data lifetime

  Assume a conservative purchase `fee` of **0.001 BTCPC = 10,000,000 dreams**
  per query batch (~one weight-unit base fee — buyers pay real money for
  aggregated intelligence, so this is deliberately modest). Owner keeps 80% =
  **8,000,000 dreams per purchase**. Creation fee is **2,000 dreams per
  reading**. Modeled over a **90-day** data-useful lifetime (mainnet quarter);
  30s epochs → **259,200 epochs**:

  | Sensor type | Readings/epoch | Lifetime readings | Creation earnings (2,000 ea) | Plausible query rate | Lifetime purchases | Usage earnings (8,000,000 ea) | Usage ÷ creation |
  |---|---|---|---|---|---|---|---|
  | `continuous` (temp/RH) | 2 | 518,400 | 1,036,800,000 dreams (0.104 BTCPC) | 1 buy / 6 h → 360 | 360 | 2,880,000,000 dreams (0.288 BTCPC) | **2.78×** |
  | `sampled` (GNSS) | 30 | 7,776,000 | 15,552,000,000 dreams (1.555 BTCPC) | 1 buy / h → 2,160 | 2,160 | 17,280,000,000 dreams (1.728 BTCPC) | **1.11×** |
  | `event` (Flipper NFC/IR) | 0.2 | 51,840 | 103,680,000 dreams (0.0104 BTCPC) | 1 buy / 30 min → 4,320 | 4,320 | 34,560,000,000 dreams (3.456 BTCPC) | **333×** |

  Usage-payment earnings exceed creation-fee earnings for **all three** types.
  The margin is thinnest for the highest-volume/lowest-value type
  (`sampled`/GNSS at 1.11×) and enormous for the low-volume/high-value type
  (`event` at 333×) — exactly the ordering we want: the sensor that spews the
  most raw readings gets the least creation subsidy per unit of value, and the
  only way to out-earn is to produce data people actually *query*. If GNSS
  query demand is lower than modeled (say 1 buy / 6 h like temp), creation
  would edge ahead for that one type — the fairness item's reputation
  multiplier and any per-type creation-fee cap absorb that tail; the flat
  2,000-dream fee keeps even the worst case near break-even rather than a spam
  profit center. **A pure spammer** (no buyers ever) earns only
  `2,000 × readings` dreams while paying real hardware/energy per fabricated,
  signed reading — net negative by construction, since 2,000 dreams
  (0.0000002 BTCPC) does not cover the cost to produce one reading.

  ##### (4) Reasoning (summary)

  - Creation should be a **cost-recovery stamp**, not an income stream:
    minting real earnings at creation rewards *volume of bytes*, which is
    trivially sybil-farmable and misaligns with data *value*. Usage payment
    rewards *bytes people pay to read*, which is the actual product.
  - A **flat, sub-cost** creation fee makes spam strictly unprofitable without
    reputation doing the heavy lifting — reputation then only handles the
    harder grey-zone (plausible-but-low-value data), not the obvious flood.
  - Fixing the split to **80/15/5** pays the storage layer out of every
    purchase (aligning "everything is compute" from Phase 2) instead of leaving
    storage entirely dependent on the separate emission pool.

  ##### Handoff to the implementation item ("Implement the split in `chain.rs`")

  1. Add `SENSOR_CREATION_FEE_DREAMS = 2_000` to `emission.rs`; change the
     `SensorReward` magnitude at `chain.rs:1936` to the flat fee (keep the
     entry + per-owner grouping plumbing).
  2. Fix `SensorDataPurchase` at `chain.rs:1981` to the real three-way 80/15/5
     split (add the missing `storage_share` credit); resolve the
     storage-contract account id.
  3. Leave the `rep_mult` multiplier hooks as no-ops (× 1) until the fairness
     item lands — the two plug-in points are named above.
  4. Tests (owned by the implementation item, not this doc item): spam
     scenario is net-negative; a high-query sensor out-earns a low-query one
     over N epochs.

  ---
- [x] **Fairness-to-seller design (reputation-gated).** Done — see decision
  block inline below. A sensor owner who
  reports garbage/spam data must not out-earn one reporting high-quality,
  frequently-queried data under a flat creation fee. Tie the creation fee
  and/or purchase-split percentage to a **sensor reputation score** (ties
  into Phase 6 reputation layer — do not build two separate reputation
  systems). Adversarial check: what stops someone spamming thousands of
  low-effort readings to farm creation fees? (Likely answer: creation fee
  must be below cost-to-produce for a rational spammer, real usage payment
  must dominate lifetime earnings — prove this with numbers, not intuition.)

  <!-- DECISION BLOCK (design-only; 2026-07). Author: Shin Devlin. -->

  #### Decision: reputation-gated fairness overlay (design-only)

  **Scope guard.** The *absolute* creation-fee amount and the
  `SensorDataPurchase` split percentages are owned by the sibling item above
  ("creation-fee vs. usage-payment split"). This block does NOT set those
  numbers. It defines only the **reputation-gating overlay** that sits on top
  of whatever base numbers that item picks, using symbols for them:

  - `F_base` — the base creation fee minted by `SensorReward` per accepted
    reading (in dreams; today `chain.rs` `LedgerEntry::SensorReward` is a flat
    `credit(node_id, amount)` with no gating — that's the line this overlay
    modifies).
  - `S_owner` — the owner's base share of a `SensorDataPurchase` fee (today the
    hard-coded 80% at `chain.rs` ~1981; recycle takes the remainder).

  The overlay is a pure multiplier on each, driven by one reputation score.

  ##### Gating decision (what we gate, and how)

  1. **Gate the creation fee HARD (multiplicative, floored at zero).**
     Effective creation credit becomes
     `F_eff = F_base × g_create(r)`, where `r` is the sensor's reputation
     score in `[0.0, 1.0]` (see the Phase-6 interface below) and `g_create` is
     a monotonic ramp:
     - `r < r_min` → `g_create = 0` (no creation fee at all — a brand-new or
       bad-reputation sensor earns **nothing** at creation);
     - `r_min ≤ r < r_good` → `g_create` ramps linearly from a small floor
       `g_floor` (e.g. 0.1) up to 1.0;
     - `r ≥ r_good` → `g_create = 1.0` (full base fee).

     This is the key anti-farming lever: creation income is *earned by
     reputation*, and reputation (per the Phase-6 interface) can only rise
     from **actual paid queries and corroboration**, never from emitting more
     readings. A spammer with no buyers is pinned at `r ≈ 0` and collects
     `F_eff = 0` forever.

  2. **Gate the purchase split SOFTLY (bounded multiplier).**
     Owner share becomes `S_eff = clamp(S_owner × g_split(r), S_min, S_owner)`,
     where `g_split(r)` runs from a reduced factor (e.g. 0.6) at `r = 0` to
     `1.0` at `r ≥ r_good`. The shortfall `S_owner − S_eff` goes to the
     existing recycle fund (no new sink, no new entry type). Low-rep sensors
     that *do* manage to sell still get paid, but a smaller cut — so honest
     high-rep sensors strictly dominate on both axes. `S_eff` never exceeds the
     base `S_owner` (reputation cannot let anyone *over*-earn vs. the sibling
     item's ceiling).

  Why both, not one: gating only the split leaves creation-fee farming open;
  gating only creation removes the ongoing incentive to keep a
  frequently-queried sensor honest. Gating both closes the farm and preserves
  the "usage dominates lifetime earnings" property below.

  ##### Required Phase-6 interface (this item is a CONSUMER, not a 2nd system)

  Per Phase 6 ("build ONE system; every vertical reads from it"), this overlay
  MUST NOT compute or store its own trust number. It requires Phase 6 to expose
  a single read-only reputation primitive with **exactly** this contract, and
  Phase 6 is hereby required to satisfy it:

  - **Key:** `sensor_id` (the `String` on `SensorReading`/`SensorDataPurchase`),
    NOT the owner account. Rationale: an owner may run one excellent sensor and
    one spam sensor; reputation must be per-device so the spam sensor cannot
    ride the good one's score. Phase 6's account-keyed score is the *input*;
    Phase 6 must additionally expose a per-`sensor_id` projection derived from
    the same signals (purchase history, complaint/`OrderDispute` history,
    corroboration) — one code path, two lookup keys, not two systems.
  - **Range:** normalized `r ∈ [0.0, 1.0]` (fixed-point on-chain, e.g.
    basis points `0..=10_000`, to stay deterministic — no floats in consensus
    state).
  - **Signals (already on-chain, no new gamed input):** count/BTCPC-volume of
    distinct-buyer `SensorDataPurchase` events for this `sensor_id`;
    corroboration (independent devices agreeing, as `CoverageReport` already
    rewards); complaint/dispute history. All are things Phase 6's design bullet
    already lists.
  - **Decay:** time-decay toward a low baseline when a sensor stops being
    queried, so parked-but-formerly-good sensors can't hold max rep costlessly.
    Phase 6 sets the half-life; this item just requires that decay exists.
  - **Sybil resistance is Phase 6's job, inherited for free:** distinct-buyer
    weighting (not raw purchase count) and stake/age-weighting live in Phase 6
    so wash-querying yourself doesn't pump `r`. This overlay assumes that and
    does not re-implement it.

  Until Phase 6 ships, treat `r` as a stub returning `r_good` for
  genesis/whitelisted sensors and `0` otherwise, so the overlay is testable in
  isolation and fails **closed** (unknown sensor → no creation fee).

  ##### How would someone game this? (PRD ground rule 3)

  **Attack 1 — spam thousands of low-effort readings to farm the creation fee.**
  This is the headline attack and the overlay must make it strictly
  unprofitable. Proof with numbers (using the overlay's own logic, independent
  of the sibling item's exact `F_base`):

  - A fresh spam `sensor_id` has no purchase history and no corroboration → its
    Phase-6 score is `r ≈ 0 < r_min` → `g_create(r) = 0` →
    **`F_eff = F_base × 0 = 0` dreams per reading.** The spammer earns nothing
    at creation no matter how many readings they emit. Farming the creation fee
    is arithmetically impossible while `r < r_min`, which is exactly the state
    a no-buyer spammer is stuck in.
  - To lift `r` above `r_min` the spammer must generate *real, distinct-buyer,
    paid* `SensorDataPurchase` events — i.e. become a legitimate seller. Faking
    those by self-buying is defeated in Phase 6 by distinct-buyer + stake/age
    weighting (required above); each sock-puppet buyer needs funded,
    aged/staked accounts and pays real fee (the recycle share is burned to the
    fund, a net loss to the attacker on every wash purchase).
  - **Cost-to-produce vs. reward, concretely.** Even ignoring gating, a
    rational spammer's marginal cost to produce one accepted reading is bounded
    below by: (a) the transaction/anti-sybil cost of getting the entry accepted
    (the node already enforces hardware/machine-id anti-sybil and the
    no-peers-no-submit hardline), and (b) storage of the blob. The sibling item
    is required to set `F_base` **below** that marginal cost-to-produce
    (that is *its* acceptance criterion; we restate it here as a hard
    dependency). With `F_base < cost_to_produce` and `g_create = 0` for spam,
    the spammer's per-reading margin is `0 − cost_to_produce < 0`. Thousands of
    readings multiply a negative number: **more spam = more loss.**
  - Worked example (illustrative units, real numbers set by sibling item):
    say `cost_to_produce ≈ 50` dreams/reading (accept + store) and the sibling
    item sets `F_base = 20` dreams (below cost, as required). A spammer emitting
    10,000 readings with `r≈0`:
    creation income `= 10_000 × (20 × 0) = 0` dreams;
    cost `= 10_000 × 50 = 500_000` dreams; net **−500,000 dreams.**
    Even in the impossible case where gating were absent
    (`g_create = 1`): income `= 10_000 × 20 = 200_000` < cost `500_000` → still
    net **−300,000 dreams.** The gate turns a merely-unprofitable attack into a
    zero-income one.

  **Attack 2 — slow-burn: build a real sensor's rep, then dump spam under the
  same `sensor_id`.** Defeated by per-`sensor_id` decay + complaint signal: a
  previously-good sensor that starts emitting garbage draws disputes/complaints
  (a Phase-6 negative signal) and stops earning corroboration, so `r` decays
  back down and `g_create` collapses toward the floor/zero. The attacker cannot
  "cash out" accumulated creation-fee rate because creation fee is paid
  *per current reading at current `r`*, not banked.

  **Attack 3 — wash-query your own sensor to pump `r` and unlock full split.**
  Not defended here — explicitly delegated to Phase 6 (distinct-buyer counting,
  stake/age weighting, wash-trading defense are enumerated in Phase 6's design
  bullet). This overlay only *consumes* the resulting `r`. If Phase 6's `r`
  is honest, `g_split`/`g_create` are honest. Stated as a hard dependency so we
  don't build a second, weaker defense here.

  ##### Why a legitimate high-query sensor out-earns a garbage/low-query one

  Lifetime earnings of a sensor:
  `E = Σ_readings (F_base × g_create(r))  +  Σ_purchases (fee × S_eff(r))`.

  - **Garbage / low-query sensor:** few or zero purchases → `r` stays low →
    `g_create → 0` (little/no creation income) AND `g_split` reduced on the rare
    sale. Both terms are suppressed. Lifetime `E ≈ 0`.
  - **High-quality, frequently-queried sensor:** many distinct-buyer purchases
    → `r → r_good` → `g_create = 1` (full creation fee) AND `g_split = 1` (full
    owner share) AND the purchase term itself scales with query volume. The
    second sum (`Σ purchases`) is by construction the dominant term over the
    sensor's lifetime — this is the sibling item's "usage payment must dominate"
    requirement, which the overlay preserves rather than fights: reputation
    *amplifies* the already-dominant usage term for good sensors and *zeroes
    the creation term* for bad ones. There is no ordering of `r` under which a
    low-query sensor's `E` exceeds a high-query sensor's `E`, because every
    gate is monotonic non-decreasing in `r` and `r` is monotonic
    non-decreasing in genuine paid usage.

  ##### Left for the 1.1 implementation item (NOT built here — no tests run,
  this is a design-only decision block)

  - Modify `chain.rs` `LedgerEntry::SensorReward` to multiply the credit by
    `g_create(r)` (look up `r` by `sensor_id` via the Phase-6 primitive);
    modify the `SensorDataPurchase` split (~line 1981) to apply `g_split(r)` to
    the owner share and route the shortfall to `RECYCLE_FUND_ACCOUNT`.
  - Pin the overlay constants (`r_min`, `r_good`, `g_floor`, `S_min`, and the
    basis-point representation) as named consts alongside the emission
    schedule, once the sibling item fixes `F_base`/`S_owner`.
  - **Tests to add in the implementation item (do NOT write here):**
    (1) spam scenario — N readings from a fresh/low-rep `sensor_id` credit
    exactly 0 and net-loss vs. modeled `cost_to_produce`;
    (2) high-query sensor out-earns a low-query one over N epochs;
    (3) slow-burn — rep decays and `g_create` collapses after complaints;
    (4) fail-closed — unknown `sensor_id` yields `g_create = 0`.
  - **Phase-6 dependency to file:** add the per-`sensor_id` projection and the
    `[0, 10_000]`-bp read API to the Phase-6 design bullet as a required output
    of that single reputation system.
- [ ] **Implement the split in `chain.rs`** — adjust `SensorReward`
  application to the small-fee model; confirm `SensorDataPurchase`'s
  existing split logic matches the design. Add tests: spam-reporting
  scenario should be unprofitable; legitimate high-query sensor should
  out-earn a low-query one over N epochs.

### 1.2 — Universal sensor ingest (any sensor type, no new chain code per type)

- [x] **Audit current sensor entry coverage** — `SensorReading` (generic),
  `CoverageReport` (cellular dead-spot specific). Confirm the generic
  `SensorReading.metadata: Option<serde_json::Value>` field is sufficient
  for arbitrary sensor payloads (GNSS position+accuracy, sub-GHz signal
  capture, Flipper's IR/NFC/RFID/BLE reads, Helium miner witness data) or
  whether a typed schema-per-sensor-class is needed. Decide and document.
  **Done — see audit + decision block inline below (generic metadata kept,
  JSON convention documented).**

  **RESOLVED — user directive: fix this now, treat as urgent (2026-07-01).**
  See "SECURITY FIX: Sensor entry authentication" design block immediately
  below. This supersedes the open question that was here.

  ---

  #### SECURITY FIX: Sensor entry authentication (urgent, Phase 1.1a)

  **The vulnerability, confirmed by direct code read (not just the audit's
  description):**
  - `rust/btcpc-node/src/tx.rs` lines ~465-490 put `SensorReading`,
    `SensorRegister`, and `GatewayHeartbeat` in a literal "Allowlisted
    pass-through" match arm with **zero signature verification** — entries
    go straight to `chain.apply_entry()`.
  - `chain.rs:1014` (`SensorReading` application) destructures
    `sensor_id, metadata, epoch` and explicitly **discards `owner`** via
    `..` — the only validation present is a GNSS-speed plausibility check
    (`SENSOR_GNSS_MAX_SPEED_M_S`). `value` and `data_hash` are never
    checked against anything.
  - `main.rs:1929-1950` computes `SensorReward` (and `GatewayRewardSplit`)
    payouts by iterating `sensor_nodes`/`sensor_gateway_map`, which are
    built from these unauthenticated entries. **Confirmed exploit path:**
    submit a `SensorReading` (or `SensorRegister`/`GatewayHeartbeat`) with
    any `owner`/`node_id` string you like — including someone else's real
    account — and that epoch's `SensorReward` (or `GatewayRewardSplit`)
    pays out to whatever account you named. No signature, no stake, no
    prior registration required.

  **Why this isn't a one-line fix — struct-level asymmetry found during
  design:**
  - `SensorRegister` and `GatewayHeartbeat` **already have a `signed_by:
    AccountId` field** in their struct definitions
    (`crates/btcpc-types/src/entry.rs`) — the field exists, `check_signature`
    (the exact mechanism already used for `Stake` et al., see `tx.rs:2111`)
    is simply never called on them. Fixing these two is a small, non-breaking
    `tx.rs` change: move them out of the pass-through arm, call
    `check_signature(chain, signed_by, entry, sig_hex, "posting")` (posting
    key — device-tier auth, not owner/active — matches the "any sensor
    device, not a hot wallet" model used elsewhere in the sensor design).
  - `SensorReading` **has no `signed_by`, `sig_hex`, or `nonce` field at
    all.** This is the genuinely breaking part: adding real per-reading
    authentication requires an `entry.rs` schema change to `LedgerEntry`,
    which every node on the network must run to keep parsing new blocks,
    AND every producer of `SensorReading` must be updated to actually sign.
  - **Confirmed producers requiring updates**, found by grepping the whole
    repo for `LedgerEntry::SensorReading` construction:
    - `rust/btcpc-node/src/sim.rs` — test/simulation harness. **Correction
      during implementation:** `main.rs` was initially believed to also
      construct `SensorReading` (the `BTCPC_SENSOR` env var it documents is
      real, but re-checked by direct grep during implementation — `main.rs`
      does not actually construct a `SensorReading` entry anywhere; only
      `sim.rs` and the Android client do). Removed from the producer list.
    - `rust/btcpc-android/src/sensors.rs` — **the live Android sensor
      client. Confirmed it has NO account-key signing capability today.**
      Verified precisely during implementation: `rust/btcpc-android` DOES
      have an ed25519 `Keypair` in `net.rs`, but it's a **libp2p transport
      identity** (peer-to-peer networking/handshake identity), not a
      BTCPC account posting key that `check_signature` would recognize —
      those are registered on-chain per `AccountId` via
      `AccountUpdateKey`/`SensorKeyRegister`, which this crate has no
      account/wallet module for at all (confirmed: no `account.rs`,
      `wallet.rs`, or equivalent exists under `rust/btcpc-android/src/`).
      **This means wiring real signing into the Android client is separate,
      larger feature work** (deriving/storing a posting key, exposing it to
      `sensors.rs`, signing the canonical entry hash, submitting `sig_hex`
      alongside the gossiped entry) — scoped OUT of this urgent fix and
      tracked as its own follow-up item below. `sensors.rs` also calls
      `chain.apply_entry()` directly (local self-apply before gossip
      broadcast), bypassing `tx.rs`/`validate_and_apply` entirely on-device
      — the new signature check only bites when OTHER nodes receive this
      entry via gossip and validate it through the normal path, which is
      exactly where the theft vector lived (a remote attacker forging
      entries for someone else's account). The phone's own local apply of
      its own genuine readings was never the risk.
    - **Follow-up item (not urgent, tracked separately):** add real BTCPC
      posting-key signing to `rust/btcpc-android` so genuine phone-submitted
      readings can pass the "owner has a posting key" branch of the new
      check instead of relying on the bootstrap-skip path forever. Until
      this lands, phone sensor owners should avoid registering a posting
      key on their sensor-owner account if they want their phone's own
      readings to keep applying — or accept that their readings will start
      being rejected by remote nodes the moment they do register one. This
      tradeoff is inherent to shipping the chain-side fix first; flagging
      it here so it isn't a surprise later.

  **Rollout plan (why this can't be a silent merge-to-main):**
  1. **Schema change — corrected during implementation**: only
     `signed_by: AccountId` needs to be added to
     `LedgerEntry::SensorReading` in `crates/btcpc-types/src/entry.rs`. The
     actual signature bytes do NOT need to live in the struct — confirmed
     by reading how `Stake` (which has `signed_by` but no in-struct
     signature field) actually gets verified: the signature travels
     out-of-band as an HTTP-layer parameter, threaded through
     `validate_and_apply(chain, entry, sig_hex: Option<&str>)` at the API
     boundary (`api.rs` extracts it per-request, e.g. via
     `canonical_signing_message` + a submitted `sig_hex` field on the HTTP
     body). So this is a smaller change than first designed: one new
     struct field (`signed_by`), no `sig_hex` field on the entry itself.
     "Make it optional for compatibility" still applies to how `tx.rs`
     TREATS `signed_by`/verification (step 2's bootstrap-skip), not to the
     struct shape.
  2. **tx.rs validation, soft-launch mode**: move `SensorReading` out of
     the pass-through arm. New logic: if the named `owner` account has NO
     posting key registered yet (fresh/unregistered sensor — the common
     case for a brand-new device), **allow through unsigned** (mirrors
     `check_signature`'s own existing "key not set yet — skip" bootstrap
     behavior at `tx.rs:2111`, so this reuses existing semantics, doesn't
     invent new ones). If the `owner` account DOES have a posting key
     registered, `sig_hex` becomes mandatory and must verify — this is
     what closes the theft vector for every account that matters (anyone
     with an existing balance/reputation to protect already has a key
     registered from normal account use).
  3. **Same treatment for `SensorRegister`/`GatewayHeartbeat`** — wire
     `check_signature` using their existing `signed_by` field, same
     bootstrap-skip behavior for brand-new device-owner accounts.
  4. **Update all three producers** to sign when the owner account has a
     posting key available: `main.rs`, `sim.rs`, and — the one with real
     user impact — `btcpc-android/src/sensors.rs` needs an actual signing
     path added (device posting key must be available on-device; check
     how the Android app currently manages any chain keys at all before
     assuming one exists to reuse).
  5. **Sequencing**: this is NOT "merge to main and walk away." Step 1-3
     (chain-side) can land and deploy first because of the bootstrap-skip
     compatibility path — old unsigned readings from accounts with no
     posting key keep working, so this is non-breaking on day one. Step 4
     (client-side signing) then rolls out after, and its real security
     value only kicks in once (a) clients are actually signing and (b)
     accounts register posting keys. Until posting-key registration is
     widespread among real sensor-owner accounts, the vulnerability is
     narrowed (can't forge readings FOR accounts that already have a key)
     but not fully closed (fresh/keyless accounts remain exploitable by
     design, matching how every other bootstrap-skip entry type in this
     codebase already behaves).
  6. **Tests required**: (a) unsigned reading for a keyless owner still
     applies (bootstrap case, no regression); (b) unsigned or
     wrong-signature reading for an owner WITH a registered posting key is
     rejected; (c) correctly-signed reading for a keyed owner applies;
     (d) same three cases for `SensorRegister` and `GatewayHeartbeat`.
  7. **Explicitly deferred, not forgotten**: this fix does not address
     `data_hash`/`value` integrity (could still submit a signed but
     fabricated reading) — that's a data-quality/reputation problem for
     Phase 6, not an authentication problem. This item is scoped ONLY to
     "is the claimed owner the one who actually submitted this," which is
     the theft-of-funds vector.

  ---

  #### AUDIT + DECISION (Phase 1.2, resolved)

  **Decision: KEEP generic `SensorReading.metadata`. Do NOT add a typed
  schema-per-sensor-class enum variant.** Adopt a documented JSON convention
  (below) as the contract, enforced by the aggregation service + device keys
  — NOT by new chain code. Rationale and the one real gap follow.

  **Ground truth audited (files read, not assumed):**
  - `crates/btcpc-types/src/entry.rs` — `SensorReading { sensor_id, owner,
    epoch, value: f64, data_hash, metadata: Option<serde_json::Value> }`
    (~L390). No `signed_by` field. `CoverageReport` (~L404) is the only
    typed sensor class and is cellular-specific (lat, lon, signal_dbm,
    carrier_mcc_mnc, technology, data_hash). `SensorRegister` (~L771) and
    `SensorDataCommit` (~L795) carry their own `sensor_type: String` +
    `metadata`/`batch_hash` and ARE signed (`signed_by` / device key).
  - `src/chain.rs` — `SensorReading` apply (~L1014) treats `metadata`
    opaquely except an optional `lat`/`lon` GNSS plausibility check
    (`haversine_m` vs `SENSOR_GNSS_MAX_SPEED_M_S = 300 m/s` over one epoch).
    `SensorRegister` apply (~L1487) just persists `sensor_type` + `metadata`
    verbatim to the `sensor:{id}` meta key for API queries.
  - `src/tx.rs` — `SensorReading` is in the allowlisted **pass-through** arm
    (~L466), applied with **no signature check at all**. The signed sensor
    path is `SensorDataCommit` (signed by device key) + `SensorRegister` /
    `SensorKeyRegister` / `DeviceKeyRegister`.

  **Per-sensor-class payload analysis — can opaque JSON carry it losslessly
  and verifiably?**

  | Sensor class | Payload it needs | Fits in `metadata` JSON? | Notes |
  |---|---|---|---|
  | **GNSS position+accuracy** | lat, lon, alt_m, h_accuracy_m, v_accuracy_m, fix_type, sat_count, hdop, timestamp | Yes | `lat`/`lon` keys are already read by the chain's plausibility check — this class is the de-facto reference convention. |
  | **Sub-GHz signal capture** | freq_hz, rssi_dbm, bandwidth_hz, modulation, protocol, raw capture (large) | Yes for scalars; raw capture goes off-chain, referenced by `data_hash` | Numeric floats/strings only — trivially JSON. |
  | **Flipper IR / NFC / RFID / BLE** | protocol/tag_type, uid/card_id, key/sector data, freq, raw dump (large) | Yes for identifiers/scalars; raw dumps off-chain via `data_hash` | Heterogeneous but all string/number/bool → native JSON. |
  | **Helium miner witness** | witness_pubkey, challengee, rssi, snr, freq, datarate, timestamp, packet_hash | Yes | Already a JSON-shaped attestation upstream; maps 1:1. |

  Every named class reduces to (a) a small set of scalar/string fields that
  are natively representable in `serde_json::Value`, plus (b) optional bulk
  binary (RF captures, NFC dumps) that does **not** belong on-chain and is
  already handled by the existing off-chain-blob + `data_hash` pattern. There
  is **no** class in scope whose *on-chain* payload cannot be expressed
  losslessly as JSON. A typed enum-per-class would therefore add one chain
  variant + `tx.rs` + `chain.rs` code per sensor type — directly violating
  the Phase 1.2 goal ("no new chain code per type") — while buying nothing
  that a documented convention + off-chain-blob doesn't already give us.
  `CoverageReport` stays typed because it is load-bearing for **consensus-side
  logic** (grid-cell quantization at `COVERAGE_GRID_RESOLUTION`, corroboration
  bonus, dead-spot reward multipliers) that the chain itself must compute;
  ordinary sensor classes have no such consensus logic and must not acquire a
  bespoke variant just to be aggregated.

  **JSON convention (the contract Phase 1.2 aggregation + Phase 1.3 firmware
  build against).** `metadata` is a JSON object. Reserved top-level keys:

  - `class` (string, REQUIRED) — sensor-class discriminator. Enum:
    `"gnss" | "subghz" | "ir" | "nfc" | "rfid" | "ble" | "helium_witness"`.
    New classes are added by convention here, never by a chain change.
  - `schema` (string, REQUIRED) — convention version, e.g. `"1"`. Bump on any
    breaking key change so the aggregator can branch.
  - `lat`, `lon` (number, degrees WGS-84, OPTIONAL) — **must** use exactly
    these key names when a geolocation is present: the chain already reads
    them for GNSS plausibility, and the aggregator groups by geography on
    them. Do not nest or rename.
  - `ts_ms` (number, OPTIONAL) — device-side capture time (epoch ms), for
    sub-epoch ordering inside the aggregator.
  - `payload` (object, REQUIRED) — class-specific fields, namespaced so
    classes never collide:
    - `gnss`: `alt_m, h_accuracy_m, v_accuracy_m, fix_type, sat_count, hdop`
    - `subghz`: `freq_hz, rssi_dbm, bandwidth_hz, modulation, protocol`
    - `ir`: `protocol, address, command`
    - `nfc`: `tag_type, uid, atqa, sak`
    - `rfid`: `tag_type, id_hex, freq_khz`
    - `ble`: `mac, name, rssi_dbm, service_uuids, mfg_data_hex`
    - `helium_witness`: `witness_pubkey, challengee, rssi, snr, freq_hz,
      datarate, packet_hash`
  - Bulk binary (RF I/Q, NFC full dumps) is NOT inlined. Store off-chain,
    put its SHA-256 in the entry's top-level `data_hash`, and reference it
    from `payload` as `blob_ref` if needed. The top-level `value: f64` stays
    the single scalar the class considers primary (rssi, accuracy, etc.) so
    existing scalar tooling keeps working.

  **Verifiability concern (the one real gap — flagged, not rubber-stamped).**
  Per PRD ground rule 5, the audit surfaced a genuine problem that the
  convention alone does NOT solve: **`SensorReading` is applied with no
  signature verification** (`tx.rs` allowlisted pass-through, no `signed_by`
  field on the variant). Today `metadata` and `data_hash` are attacker-
  controlled and bound to nothing — any peer can submit a `SensorReading`
  claiming any owner, position, or payload, and the chain accepts it. The
  existing GNSS plausibility check is a weak sanity filter, not authentication.
  Because a typed enum would inherit this exact same gap (typing a field does
  not sign it), this is further evidence the fix belongs at the trust layer,
  not the schema layer. The aggregation service and Flipper firmware MUST
  therefore treat provenance as follows:

  1. **Prefer the signed path for anything that earns or is sold.** Real
     device submissions should go through `SensorDataCommit` (signed by the
     device key from `DeviceKeyRegister` / `SensorKeyRegister`), with the raw
     readings batched off-chain under `batch_hash`. `SensorReading` remains
     the lightweight, best-effort telemetry lane.
  2. **Aggregator trust rule:** a reading counts as *verified* only if its
     `sensor_id` resolves (via `sensor:{id}` state) to a sensor whose owner
     has a registered device key, AND the reading (or its batch) is signed by
     that key. Unsigned `SensorReading` events are ingested as *unverified*
     and must be visibly downweighted / excluded from paid B2B answers.
  3. **Do NOT** paper over this by adding trust to opaque metadata. The clean
     fix (tracked as a follow-up, not blocking this decision) is to require a
     device signature on reward-bearing sensor entries — see 1.1/1.3 and the
     Phase 6 reputation layer.

  **FLAG FOR USER REVIEW:** the generic-vs-typed question resolves cleanly to
  *generic*, but the audit exposed that reward-/sale-bearing `SensorReading`
  ingestion currently has **no authentication**. That is a separate,
  higher-severity issue than schema shape. Recommend the aggregation service
  (1.2) be built to the "signed = verified, unsigned = untrusted" rule above
  from day one, and that requiring a device signature on reward-bearing sensor
  entries be scheduled explicitly (touches 1.1 payment split and Phase 6
  reputation). Do not ship paid B2B answers (1.4) off unverified readings.

  ---

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

- [x] **Inventory Flipper Dolphin's onboard sensors/radios** actually worth
  exposing (sub-GHz, 125kHz RFID, NFC, infrared, BLE, GPIO) and which map to
  a Verasens-useful reading type. **Done — doc-only, decision inline below.**
  No code touched, no tests to run (see note at end of this sub-item). The
  firmware capture/submit work and the real-hardware verification remain as
  the two downstream 1.3 items below (still unticked).

  **Scope note / hardware accuracy.** This inventory covers the *stock* Flipper
  Zero (STM32WB55 MCU: dual-core Cortex-M4 + M0 with an onboard BLE 5 radio;
  CC1101 sub-GHz transceiver; ST25R3916 NFC front-end; 125 kHz RFID coil +
  T5577/EM-reader analog path; IR TX + RX; 1-Wire/iButton; 18-pin GPIO
  header). It deliberately does **not** claim capabilities the stock unit
  lacks: the Flipper Zero has **no GPS, no accelerometer/IMU, no temperature/
  humidity/pressure/gas sensor, no microphone, and no Wi-Fi** onboard. GPS and
  environmental/Wi-Fi readings are only available via **GPIO add-on modules**
  (e.g. the official Wi-Fi Devboard, or third-party GNSS/environmental
  boards) and are treated below as *GPIO-gated*, not native. Where I am not
  fully certain a capability is stock vs. module, it is called out explicitly
  rather than asserted.

  The existing prototype (`clients/btcpc-flipper`, ~240 LOC C, do not modify
  here) already anticipates most of these: its BLE wire protocol
  (`protocol/btcpc_protocol.h`) defines `SUBGHZ_OBS`, `RFID_SCAN`, `NFC_SCAN`,
  `IBUTTON`, `IR_CAPTURE`, `HEARTBEAT` frames, plus phone→Flipper `GPS` and
  `CLOCK_SYNC` — so the radio set below is consistent with what the firmware
  scaffolding already reserves message types for. The Flipper signs each frame
  with its onboard ed25519 key (STM32WB55 TRNG-seeded) and the phone relays it
  on-chain; the device pubkey is registered via `DeviceKeyRegister`
  (`device_pubkey` + optional `hardware_hash`) and stakes via
  `DeviceClaimStake`/`DeviceYieldStake`, so every `SensorReading` below is
  attributable to a staked, registered device key.

  **Radio/sensor inventory and Verasens mapping**

  | Radio / sensor | Stock? | What it can physically measure/capture | Verasens-useful? | Why |
  |---|---|---|---|---|
  | **Sub-GHz (CC1101)** | Yes | RF energy in the CC1101 ranges (~300–348 / 387–464 / 779–928 MHz): centre freq (Hz), RSSI (dBm), modulation (AM/FM/OOK), decoded remote/sensor protocols | **Yes (primary)** | RSSI-at-location is a real, corroboratable physical measurement (RF spectrum occupancy / noise-floor / signal-presence mapping). Directly analogous to the existing `CoverageReport` dead-spot model but for unlicensed ISM bands. Strongest Verasens value of the set. |
  | **NFC (ST25R3916, 13.56 MHz)** | Yes | Presence + identity of ISO-14443A/B, ISO-15693, FeliCa tags: UID, ATQA, SAK, tech class | **Yes (proximity/presence)** | A *timestamped, signed proof that a specific NFC tag was physically read by this device* is a genuine proximity/attendance/asset-checkpoint reading (feeds Phase 7.3 proof-of-location and asset-tracking). Not an environmental scalar — value is presence/identity, so it maps as an event, not a magnitude. |
  | **125 kHz RFID (coil + T5577 path)** | Yes | Presence + ID of low-freq cards (EM4100, HID Prox, Indala): protocol + card ID bytes | **Yes (proximity/presence)** | Same category as NFC — signed proof-of-read of a low-freq credential/asset tag at a place and time. Useful for access-point / checkpoint verification. Lower information density than NFC but still a valid presence event. |
  | **Infrared (TX + RX)** | Yes | Captures modulated IR remote signals (protocol + timing) on RX; the RX photodiode also responds to broadband IR, but there is **no calibrated lux/temperature sensor** | **Partial / weak** | IR *capture* (which appliance/remote is present) is a niche presence signal, marginally useful for device-inventory/occupancy. It is **not** a calibrated ambient-light or IR-temperature reading — do not represent it as one. Map only the "IR protocol seen" presence event; mark magnitude readings not-available. |
  | **1-Wire / iButton** | Yes | Reads Dallas/Maxim 64-bit ROM codes (and, with a DS18B20 probe on the 1-Wire pin, temperature — but the probe is an **add-on**, not stock) | **Yes for ID-read; add-on-gated for temperature** | iButton ROM-code read is a valid signed presence/asset event (same class as RFID). A DS18B20 on the 1-Wire line WOULD give a genuine temperature scalar, but that is a GPIO/1-Wire *module*, so temperature is flagged GPIO-gated, not native. |
  | **BLE (STM32WB55 radio)** | Yes | Onboard Bluetooth LE 5 radio (advertise/scan/connect) | **No, as a sensor — Yes as transport** | BLE here is the *transport* to the paired phone (already the prototype's data path), and could scan for nearby BLE advertisements as a crude presence/density signal. But BLE-scan-as-sensor overlaps heavily with the phone's own radios and adds little unique physical measurement. Treat BLE as transport, not a Verasens reading source (revisit only if a "BLE beacon density" reading proves valuable). |
  | **GPIO header (18-pin)** | Yes (header); sensors are add-ons | Digital/analog I/O, I²C/SPI/UART, 1-Wire, 3V3/5V rails — the attachment point for external sensors (GNSS, temp/humidity/pressure, gas, light, etc.) | **Yes, but only WITH a module** | The GPIO header is where *real environmental scalars* enter (e.g. a GNSS board → position+accuracy like `CoverageReport`; a BME280 → temp/humidity/pressure). None of these are onboard. Flag the header as the extensibility path and gate every environmental reading on "module present," never assert the stock unit measures them. |
  | **Battery / uptime (housekeeping)** | Yes | Battery %, uptime, firmware version | **No (telemetry, not sensing)** | Device-health telemetry only (already the `HEARTBEAT` frame). Not a Verasens physical-world reading; keep it as device liveness/heartbeat, not a `SensorReading`. |
  | ~~GPS / IMU / temp / humidity / Wi-Fi (onboard)~~ | **No** | — | **N/A — not onboard** | Explicitly listed to prevent invented hardware: the stock Flipper Zero has none of these. Any such reading is GPIO-module-gated (see GPIO row) or supplied by the paired phone (the prototype's phone→Flipper `GPS` frame is exactly this — position comes from the *phone*, not the Flipper). |

  **Verdict summary.**
  - **Verasens-useful (map to `SensorReading`):** Sub-GHz (primary, magnitude
    reading), NFC (presence event), 125 kHz RFID (presence event), iButton
    ROM-read (presence event), IR-capture (weak presence event only), and any
    GPIO-module-supplied environmental scalar (module-gated).
  - **Not a Verasens reading source:** BLE (it is the *transport*), battery/
    uptime heartbeat (device telemetry), and — critically — every capability
    the stock unit does not physically have (GPS/IMU/temp/humidity/Wi-Fi).

  **Mapping onto the generic `SensorReading` shape.**
  `SensorReading { sensor_id, owner, epoch, value: f64, data_hash, metadata }`
  (`crates/btcpc-types/src/entry.rs`). Convention proposed here:
  - `sensor_id` = `"{device_pubkey_prefix}:{sensor_class}"` (e.g.
    `"a1b2c3…:subghz"`), so one registered Flipper device key can expose
    multiple sensor classes under stable per-class ids.
  - `owner` = the account that registered the device (`DeviceKeyRegister.owner`).
  - `data_hash` = SHA-256 of the raw captured payload (the exact bytes the
    Flipper signed in its BLE frame), so the reading is independently
    verifiable against the signed capture.
  - `value: f64` = the single most meaningful scalar for the class; for
    presence-only classes with no natural magnitude, use `1.0` = "read
    occurred" and put the identity in `metadata`.

  | sensor_class | `value: f64` | Proposed `metadata` JSON keys |
  |---|---|---|
  | `subghz` | RSSI in dBm (e.g. `-92.0`) | `{ "freq_hz", "modulation": "AM"\|"FM"\|"OOK", "bandwidth_khz", "protocol": <decoded name or null> }` |
  | `nfc` | `1.0` (presence) | `{ "tech": "A"\|"B"\|"F"\|"V", "uid", "atqa", "sak" }` |
  | `rfid125` | `1.0` (presence) | `{ "protocol": "EM4100"\|"HID"\|"Indala"\|"raw", "card_id" }` |
  | `ibutton` | `1.0` (presence) | `{ "rom_code", "family" }` |
  | `ir` | `1.0` (presence) | `{ "ir_protocol": <name or "raw">, "captured": true }` |
  | `gnss` *(GPIO module only)* | horizontal accuracy in metres | `{ "lat", "lon", "alt_m", "module": "<board id>", "source": "gpio" }` |
  | `env` *(GPIO module only)* | primary scalar (e.g. °C) | `{ "temp_c", "humidity_pct", "pressure_hpa", "module": "<board id>", "source": "gpio" }` |

  **Dependency flagged on Phase 1.2 (metadata convention).** As of this
  writing the Phase 1.2 item *"Confirm the generic `SensorReading.metadata`
  field is sufficient … or whether a typed schema-per-sensor-class is needed —
  Decide and document"* is **still unchecked and has landed no recorded
  decision**. Therefore the `metadata` key names above are a **proposal, not a
  ratified contract**, and **must be reconciled with Phase 1.2** once that
  audit decides between (a) free-form `metadata` and (b) a typed
  schema-per-sensor-class. If 1.2 lands a per-class schema convention, these
  Flipper classes (`subghz`/`nfc`/`rfid125`/`ibutton`/`ir`/`gnss`/`env`) must
  adopt exactly its key names and types rather than the ones above. The
  presence-vs-magnitude distinction (`value = 1.0` for presence classes) and
  the `data_hash = SHA-256(signed payload)` verifiability rule should hold
  under either 1.2 outcome and are recommended as inputs to that decision.

  **No tests to run** — this is a documentation/inventory item only, no code
  was written or modified (the ~240-LOC C prototype in `clients/btcpc-flipper`
  was read for grounding but intentionally left untouched). The firmware work
  (capturing and submitting these `SensorReading` entries per class) and the
  real-hardware verification remain as the two downstream 1.3 items below.

- [ ] **Extend/replace `clients/btcpc-flipper`** (currently ~240 lines, C,
  prototype only) to capture and submit `SensorReading` entries for each
  supported sensor, signed by the device key already described in
  `DeviceKeyRegister`/`DeviceClaimStake`.
- [ ] **Test on real hardware** — no faked emulator success. A submitted
  reading must be independently verifiable on-chain and correctly ingested
  by the Phase 1.2 aggregation service.

### 1.4 — Institutional dashboard (B2B, USD billing)

- [x] **Design the institutional product**: companies log in, browse/query
  aggregated Verasens intelligence, pay in **USD** (not BTCPC) — needs a
  fiat billing integration (Stripe or equivalent), auth (proper
  login/session, not the existing bot-JWT pattern which is consumer-scale),
  and a rate plan (per-query, subscription, or both — decide and document
  in this file before building).

  **DECISION RECORD (2026-07, design-only — no code built here).** This
  record is what the dashboard, billing integration, and settlement bridge
  items below build against. The product is a thin B2B layer that sits ON
  TOP of the Phase 1.2 Verasens aggregation service and talks to it ONLY
  through that service's public query-API contract (see "Aggregation query
  contract assumed" below) — it does not read chain state or aggregation
  internals directly.

  **(1) AUTH / SESSION MODEL — org accounts + API keys + session cookies.**

  Why the existing bot-JWT pattern is inadequate: the consumer bots
  (`bots/btcpcbot`, `bots/btcpcwalletbot`) authenticate a single Telegram
  user to a single wallet with a long-lived bearer JWT minted per user.
  That model has no concept of an *organisation*, no multi-seat membership,
  no role/permission separation, no key rotation or revocation story, and
  no separation between an interactive browser session and a
  machine-to-machine credential. A `BigCo` buying sensor intelligence needs
  several analysts under one billing relationship, the ability to revoke one
  analyst without disrupting the others, and a programmatic key for their
  own data pipeline — none of which the bot-JWT gives. It is consumer-scale
  by construction and must not be reused here.

  Replacement — three distinct identity objects:
  - **Organisation account** is the billing and ownership root. It owns the
    Stripe customer (below), the rate plan, all seats, and all API keys.
    Everything a member does is attributed to the org for metering and
    billing. Data model (new B2B service DB — Postgres, NOT chain, NOT the
    consumer bot store): `org(id, name, stripe_customer_id, plan_id,
    created_at)`.
  - **Seats / members** — `member(id, org_id, email, role, status)` with
    roles `owner` | `admin` | `analyst` | `billing`. `owner`/`admin` manage
    seats, keys and plan; `analyst` can query but not change billing;
    `billing` sees invoices/usage but cannot query. Login is email +
    password with mandatory TOTP 2FA for `owner`/`admin`, or SSO (SAML/OIDC)
    for enterprise tier. Sessions are **HttpOnly, Secure, SameSite=Lax
    session cookies** backed by a server-side session store with idle +
    absolute expiry — used ONLY by the dashboard UI. No long-lived bearer
    token lives in the browser.
  - **API keys** — `api_key(id, org_id, prefix, hash, scopes, created_by,
    last_used_at, revoked_at)` for programmatic query access. Keys are
    shown once at creation, stored only as a salted hash, carry explicit
    scopes (`query:read`, `usage:read`), are independently revocable, and
    every request is rate-limited and metered per key (so an org can attach
    a key to a data pipeline and see exactly what it cost). Programmatic
    clients authenticate with `Authorization: Bearer btcpc_live_…`; the
    dashboard authenticates with the session cookie. These are two separate
    credential paths that never mix.

  Rationale: this is the standard B2B SaaS identity shape (org → members →
  API keys, cookie for humans / key for machines) precisely because it
  solves multi-seat, revocation, and human-vs-machine separation that the
  bot-JWT cannot. We deliberately do NOT put institutional buyer identity
  on-chain: the on-chain actor is the settlement account (see seam below),
  not the corporate login.

  **(2) FIAT BILLING — Stripe, metered usage + subscription.**

  Provider: **Stripe** (specifically Stripe Billing with the Meter/usage
  API + Checkout + Customer Portal + webhooks). Justification: it is the
  de-facto standard for USD SaaS billing, has first-class **metered/usage
  billing** (which we need for per-query pricing), a hosted Customer Portal
  (so we do not build invoice/card-management UI ourselves), SCA/3-DS and
  tax handling built in, and mature idempotent webhooks. An "equivalent"
  (Paddle, Chargebee, Lago) would work, but none reduces our build surface
  more than Stripe for a USD-first product, and Stripe's usage-based meters
  map cleanly onto per-query metering.

  Objects:
  - **`Customer`** — one per Organisation (`org.stripe_customer_id`). All
    charges roll up here.
  - **`Subscription`** — one per org, carrying two items: (a) a flat
    recurring **plan price** (the tier's base fee) and (b) a **metered usage
    price** tied to a Stripe **Meter** named `verasens_query`. Enterprise
    orgs may instead run on a committed-volume subscription with overage on
    the same meter.
  - **Metering per query** — the B2B service is the single source of truth
    for "what counts as one billable query." A query is billable when the
    aggregation service returns a **successful** result set; failed/empty
    /cached-identical results are not metered (documented so buyers trust
    the meter). On each billable query the service records a local
    `usage_event(id, org_id, api_key_id, member_id, query_hash, units, ts,
    stripe_meter_event_id)` row FIRST (our ledger of record), then reports a
    **Stripe Meter Event** (`event_name = verasens_query`,
    `stripe_customer_id`, `value = units`) with an **idempotency key =
    our `usage_event.id`**. Units default to 1 per query but a heavy query
    (large geography × long time window, or export) can cost N units per a
    published units-table, so pricing tracks real aggregation cost. Stripe
    aggregates the meter over the billing period and invoices automatically;
    we reconcile our `usage_event` sum against Stripe's meter total each
    period and alarm on drift. Nightly reconciliation covers any meter
    events Stripe dropped/we failed to send.

  **(3) RATE PLAN — BOTH: subscription base + metered per-query overage,
  three tiers.**

  Decision: **both**, not either/or. Pure per-query gives no predictable
  revenue and no commitment; pure subscription can't price a whale doing
  10M queries the same as a startup doing 10k. So every tier is a monthly
  base fee (which includes a bundle of query units) plus metered overage
  above the bundle, billed on the `verasens_query` meter. Concrete tiers
  (USD, launch pricing — revisit with real usage data post-launch):

  | Tier | Base / mo | Included queries / mo | Overage / query | Seats | Auth |
  |------|-----------|-----------------------|-----------------|-------|------|
  | **Starter** | $99 | 5,000 | $0.02 | 3 | pw + TOTP |
  | **Growth** | $999 | 100,000 | $0.012 | 15 | pw + TOTP |
  | **Enterprise** | custom (from $5k) | committed volume | negotiated (< $0.008) | unlimited | SSO/SAML |

  Rationale for the numbers: base-fee-per-included-query works out to
  ~$0.02 (Starter) → ~$0.01 (Growth), with overage set slightly below the
  effective included rate so heavy usage is rewarded, not punished, and so
  a buyer near a tier ceiling is nudged to upgrade rather than rack up
  overage. Overage floor stays above marginal cost (aggregation compute +
  the on-chain `SensorDataPurchase` posting cost, see seam). A "heavy query"
  units multiplier (metering above) keeps a single very expensive
  aggregation from being sold at the flat rate. Enterprise is committed
  volume because that's the segment that will run a data pipeline off an API
  key and wants a fixed annual number, not usage surprises.

  **(4) SETTLEMENT SEAM — where a USD charge triggers on-chain
  `SensorDataPurchase` (interface only; the bridge is the separate item
  below).**

  Hard requirement (PRD): USD payments must NEVER bypass on-chain
  settlement — the sensor owner is paid in BTCPC/dreams regardless of how
  the buyer paid. The seam makes the on-chain posting a mandatory,
  idempotent consequence of every billable query, decoupled from Stripe's
  invoice cycle (Stripe bills the buyer in USD monthly; the chain must pay
  the owner per purchase, immediately — these run on different clocks).

  Seam definition (this is the contract the settlement bridge implements;
  we do NOT implement the bridge here):

  - **Trigger point.** The same code path that records a billable
    `usage_event` (metering, above) also enqueues a **`SettlementIntent`**.
    A billable query → exactly one `SettlementIntent` per underlying
    sensor batch the aggregation service reports as having satisfied the
    query. (The Phase 1.2 aggregation query response is expected to include
    provenance: which `sensor_id` + `batch_hash` + `owner` contributed to
    the result — see contract assumption below.) Metering and settlement are
    written in the **same DB transaction** so a metered query can never fail
    to produce a settlement intent.
  - **`SettlementIntent` object** (B2B service DB, the bridge's inbox):
    `settlement_intent(id, usage_event_id, sensor_id, owner_account,
    batch_hash, usd_amount_cents, dreams_amount, fx_rate_id, status,
    chain_entry_hash, created_at, posted_at)`. `status`:
    `pending` → `posting` → `posted` | `failed`. `dreams_amount` is the
    BTCPC/dreams the owner must receive (1 BTCPC = 10,000,000,000 dreams,
    canonical `DREAMS_PER_BTCPC`);
    `fx_rate_id` pins the USD→BTCPC rate source/timestamp used, so the
    conversion is auditable and not re-derived later.
  - **Bridge interface (implemented by the separate "USD → BTCPC settlement
    bridge" item below).** A single async worker contract:
    `post_settlement(intent) -> Result<chain_entry_hash>` that MUST, for a
    `pending`/`failed` intent: (1) resolve `dreams_amount` from
    `usd_amount_cents` via the pinned `fx_rate_id`; (2) construct a
    `LedgerEntry::SensorDataPurchase { sensor_id, buyer, owner, batch_hash,
    fee: dreams_amount, epoch, nonce, signed_by }` — where `buyer` is the
    **platform settlement account** (a chain account the platform funds and
    controls, representing "USD-paying institutional buyer"), `owner` is the
    sensor owner from the intent, and `fee` is `dreams_amount`; (3) submit
    it through the normal chain submission path (`apply_and_broadcast` in
    `api.rs`) so it is gossiped, sealed in an epoch, and split by the
    EXISTING `SensorDataPurchase` split logic (owner majority + storage
    contract rate + recycle) — the bridge does NOT invent a new split;
    (4) on confirmed seal, write `chain_entry_hash` + `status=posted` back
    to the intent. **Idempotency:** `nonce`/`(sensor_id, batch_hash,
    usage_event_id)` uniqueness guarantees a retried intent never
    double-posts. The worker is crash-safe and self-healing per ground rule
    4: on restart it re-drives every `pending`/`posting`/`failed` intent, so
    a USD charge that has been metered but not yet settled on-chain is
    always eventually posted — a USD payment can never be "collected but
    never settled."
  - **Funding of the settlement account** (bridge concern, noted so the seam
    is complete, NOT built here): the platform pre-funds / periodically
    tops up the settlement account's BTCPC balance from treasury; USD
    revenue collected via Stripe backs that treasury. The reconciliation
    invariant the bridge must uphold: `Σ dreams posted on-chain for owners`
    corresponds to `Σ billable queries` at the pinned FX rates — surfaced on
    an internal reconciliation report. The dashboard/billing layer never
    touches chain keys; only the settlement account (held by the bridge)
    signs `SensorDataPurchase` entries.

  **Aggregation query contract assumed (Phase 1.2 dependency, not built
  here).** This design depends on the Phase 1.2 aggregation service exposing,
  per query result: (a) an authenticated query endpoint we can call
  server-side with the org's identity; (b) a deterministic notion of "one
  billable query" (success = non-empty, non-error result); and (c)
  **provenance metadata** on each result — the set of `(sensor_id,
  owner_account, batch_hash)` tuples whose data satisfied the query — so the
  settlement seam can post the correct `SensorDataPurchase`(s). If Phase 1.2
  cannot supply provenance per result, this design's settlement seam must be
  revisited (flag per ground rule 5). No dependency on aggregation internals
  beyond this public contract.

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
- [ ] **7.4 — Resilient phone light-agent / light-storage nodes.**
  **UPDATED after full GitHub review:** no dedicated phone-light-agent
  BTCPC repo exists, but `github.com/shindevlin/phonehome` (same author,
  separate project — a Rust Telegram↔Claude Code bridge) is a working,
  real precedent for the exact resilience pattern needed here: Telegram
  long-polling with a configurable interval, session-resume via a
  persisted session file, and a systemd `Restart=always` supervision
  pattern — i.e. "if it's gone, it comes back" already proven to work in
  production for a CLI-bridge use case. Study `phonehome`'s reconnect/
  session-resume design (not its code directly — different domain) as the
  starting reference for this item's resilience model, rather than
  designing reconnection semantics from zero. Core requirement, stated by
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

## URGENT infrastructure blocker (found 2026-07-01, blocks ALL future Rust work)

**`rust/btcpc-node` cannot currently be built from scratch with `cargo
check`/`cargo test` on the Beastly WSL machine (Ubuntu, user `beastly`) —
confirmed on UNMODIFIED `main`, not caused by any feature work.** This blocks
every future Phase 1-8 item that touches Rust code from running its own
tests before landing, and blocks the daily/parallel workflow's implementation
items from ever validating a `cargo test` result.

**Symptom:** `cargo check --bin btcpc-node` triggers a genuine rustc internal
compiler panic (ICE) during `resolver_for_lowering`/`check_mod_deathness`
analysis of the `api` module — not a code error, a compiler crash.

**Isolation performed (2026-07-01), all confirmed:**
- Reproduces on a completely clean scratch clone of `main` — not caused by
  the sensor-auth fix or any other in-progress change.
- Reproduces identically at rustc 1.95.0 (current `stable`) AND 1.93.0.
- rustc 1.90.0 avoids the ICE but CANNOT be used — the workspace has a
  dependency (`matrix-sdk` and friends) requiring rustc ≥1.93, so 1.90 fails
  to even start compiling with a hard version-gate error.
- **Net result: no rustc version currently on this machine, or installable
  via `rustup` at the time of this check, can both (a) satisfy the
  workspace's own dependency floor and (b) avoid the ICE.**
- The narrower `crates/btcpc-types` package (a dependency of `btcpc-node`,
  not the full binary) DOES compile cleanly — the ICE is specific to
  building the full `btcpc-node` binary crate, not a fundamental problem
  with the workspace's Rust code in general.
- Confirmed NOT a memory/resource issue (31GB RAM available, plenty free).
- Confirmed this is a **known, pre-existing problem**, not new: found
  `build.err`/`build.first.err` log files already present in the deployment
  checkout (`/home/beastly/btcpc-node/rust/btcpc-node/`) from a prior
  session's build attempts, plus a `howtoinstallandrun.md` "lessons
  learned" doc that recommends downloading a prebuilt release binary
  specifically because building from source on this machine is unreliable.
  The currently-running live node binaries were built at some point in the
  past with a toolchain/dependency combination that no longer reproduces
  from a fresh checkout today.

**What this means practically, right now:** implementation items on this
PRD can still be designed and written, but **cannot be verified by actually
running `cargo test` on this machine** until this is resolved. Do not claim
an item's tests pass without actually running them — if this blocker is
still open, say so explicitly instead (e.g. "code written, compiles per
`cargo check` on crate X, full binary test run blocked by the known rustc
ICE — see this section").

**Not resolved as part of this fix — needs its own dedicated investigation:**
- Try `cargo check` with an intermediate rustc version between 1.90 and
  1.93 if one becomes available, in case the regression is narrower than
  currently bisected.
- Check whether pinning specific dependency versions (rather than the
  compiler) resolves it — e.g. downgrade whatever exact dependency trips
  the `api` module's resolver pass, if it can be identified via `cargo
  check -Z timings` or bisecting which file triggers it.
- Consider filing the ICE upstream (rust-lang/rust) if it reproduces
  reliably — the "we would appreciate a bug report" message in the panic
  output includes a template link.
- Check if a completely fresh `rustup` install (not reusing the existing
  `~/.rustup` toolchain cache, which may have a corrupted/inconsistent
  component set) resolves it.

---

## Phase 8 — Marketing truth pass (found during GitHub-wide review)

`marketing/INNOVATIONS.md` (mirrored from `btcpc-marketing` repo) claims
several features as built that are not, per direct code verification (see
"GitHub-wide review findings" above). This phase can run in parallel with
any other phase — it's independent, low-risk, and prevents false claims
from continuing to circulate.

- [ ] **Decide per oversold claim: build it for real, or rewrite the copy.**
  For each of Lucid Pruning, Genesis Dreams/inscriptions, Sparse Merkle
  Tree state proofs, and Resource-aware mining/auto-throttle: either scope
  it as a real build item (in whichever phase above it best fits, or a new
  one) or rewrite `marketing/INNOVATIONS.md` (and `btcpc-marketing`'s copy)
  to stop claiming it exists. Do not leave it in an unverified limbo state.
- [ ] **Correct the "Finality blocks" claim precision** — either build the
  actual "every 100 epochs, full network snapshot, seconds-to-sync" feature
  described, or narrow the marketing copy to accurately describe what
  `snapshot_replication.rs` actually does (per-account snapshots) today.
- [ ] **Push corrected copy to both locations** — `marketing/*.md` in this
  repo AND the standalone `btcpc-marketing` GitHub repo must stay in sync;
  decide which is canonical (recommend: this repo's `marketing/` is
  canonical, `btcpc-marketing` becomes a mirror or is retired — document
  the decision here once made).

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

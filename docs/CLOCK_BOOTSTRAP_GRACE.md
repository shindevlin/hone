# Clock Bootstrap Grace

**Version 0.1 — July 2026**

---

## The Problem It Solves

HONE is proof-of-work: **no account is funded at genesis.** Every account starts
at zero balance and earns everything through work. (Verified: `genesis.json` has
no balance fields; the node treats a missing balance as 0.)

But clock nodes — the nodes that seal epochs and keep chain time — normally
require a **minimum stake** (`clock_min_stake`, default 5 HONE) to register. That
stake is deducted from the node's balance.

At genesis, this is a **deadlock**: every account has 0 balance, so no clock node
can post the 5-HONE stake, so no clock node can register, so **no epoch can be
sealed, so the chain never advances.** The staking requirement is correct for a
running chain but impossible to satisfy at launch, when no HONE exists yet.

## The Fix: A Bootstrap Grace Window

For the **first 100,000 epochs** from genesis (epoch 0 through 100,000 — "the
first 100k blocks", ≈34.7 days at 30 s/epoch), clock registration is bootstrapped:

1. **Register at zero stake.** During grace, `ClockNodeRegister` is accepted with
   `stake = 0` — no minimum is enforced and nothing is deducted (there is nothing
   to deduct). The node becomes an eligible clock immediately.
2. **Stake builds automatically from earnings.** While a node's clock stake is
   below the minimum, each `ClockReward` it earns is added to its **clock stake**
   (not its spendable balance) until the stake reaches `clock_min_stake` (5 HONE).
   Once at the minimum, further rewards flow to the node's balance normally.
3. **The node self-stakes through work.** By earning, a bootstrap clock organically
   accumulates its own stake — no account was ever pre-funded, and no manual stake
   transaction is needed at launch. Stake is *earned*, never granted. This is fully
   POW-consistent.

After epoch 100,000 the grace ends:
- New `ClockNodeRegister` must meet the normal `clock_min_stake` (standard rule).
- The auto-accumulate-into-stake behavior **stops** — rewards flow to balance as
  usual for all clocks.
- A clock still below the minimum stake after grace is subject to the normal
  eligibility rule (top up or lose eligibility), same as any under-staked node.

---

## Exact Rules (consensus — every node MUST compute these identically)

Let `E` be the epoch an entry applies at, and `GRACE_END = 100_000`.

**ClockNodeRegister at epoch `E`:**
- If `E <= GRACE_END`: accept with `stake = 0` (or any offered amount); do NOT
  enforce `clock_min_stake`; do NOT debit balance. Record the node with its
  current stake.
- If `E > GRACE_END`: enforce `stake >= clock_min_stake` and debit as today.
  (Re-registration keeps existing recorded stake, unchanged from current behavior.)

**ClockReward to a clock node at epoch `E`:**
- If `E <= GRACE_END` **and** the node's recorded clock stake `< clock_min_stake`:
  add the reward to the node's clock **stake** (in `clock_reg`), capped so stake
  does not exceed `clock_min_stake`; any remainder above the cap goes to balance.
- Otherwise (after grace, or stake already at minimum): credit the reward to the
  node's **balance** as today.

`GRACE_END` is a fixed constant derived from genesis (epoch 0), so it is part of
consensus from block 0 — it does not depend on wall-clock or launch time. Because
genesis is backdated (July 4), some early epochs elapse before real nodes come
online; that is fine — nobody earns until nodes are actually running, and the
"first 100k blocks" framing is what matters for explanation.

---

## Why This Is Safe

- **No pre-funding.** Genesis still funds no one. Stake is accumulated from
  ClockReward — earned work, not an allocation. POW-pure.
- **No deadlock.** Clocks can register at zero and start sealing from epoch 0, so
  the chain advances from launch.
- **Self-terminating.** The special behavior is bounded to `E <= 100_000` and to
  stake below the minimum — it cannot persist or be gamed after the window.
- **Deterministic.** `GRACE_END` is a constant; every node applies the identical
  rule to the identical entries, so the block-0 hash and all subsequent state are
  reproducible across founders (no fork risk from this rule).
- **Genesis unchanged.** This is apply-time reward/stake logic, not a genesis.json
  change — the block-0 hash is unaffected.

---

## Implementation Notes (after sign-off)

- `GRACE_END: u64 = 100_000` constant in `hone-types` (single source of truth).
- `chain.rs` `ClockNodeRegister` arm: gate the min-stake `ensure!` + `debit` on
  `epoch > GRACE_END`.
- The `ClockReward` application path (main.rs epoch-seal reward wiring / chain.rs):
  during grace + under-min, route the reward into `clock_reg` stake (capped at
  `clock_min_stake`) instead of balance.
- Tests: register-at-0 accepted pre-grace; min-stake enforced post-grace; reward
  builds stake to exactly the minimum then spills to balance; block-0 hash
  unchanged.

---

_No account is funded. Clocks earn their stake into existence during the first
100k blocks, then the normal staking rule takes over._

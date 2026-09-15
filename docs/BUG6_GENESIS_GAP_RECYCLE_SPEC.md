# Spec: genesis-gap recycle (fold into the BUG 6 cold-start fix)

**Status:** spec for grouchly to integrate into the BUG 6 cold-start/recycle rewrite. Do NOT land as a separate edit to the reward driver — it touches the exact lines BUG 6 is rewriting, and two independent edits would collide or double-credit.

**Author routing:** hand to grouchly via the pc-agent-bridge channel. Commit author `Shin Devlin <shindevlin@proton.me>`, no AI attribution, HONE branch only, never merge to main.

## Problem

HONE's genesis timestamp is fixed at 2026-07-04 (`1783191600000`) but the chain goes live weeks later, so it boots at epoch `F ~= 66,500` (the first quorum-sealed epoch), not 0. The supply cap is accounted by epoch number in `block_reward_at` (no cumulative-supply state counter). The reward driver aligns its floor to `cur - 64` and cold-start-skips forward to `first_final`, so epochs `1..F` are **never paid and never recycled** -> their rewards (`~(F-1) x 2 HONE ~= 133,140 HONE`) are forfeited. Requirement: those unearned rewards must flow to `__recycle_fund__` (the perpetual end-of-chain pool), automatically and deterministically, so realized supply still reaches 42M — matching the whitepaper's "unearned -> future tokens" rule and the way live empty epochs already recycle (`main.rs` empty-sealer branch).

## Helper (already landed, math-only, no fork surface)

`crates/hone-types/src/emission.rs`:

```rust
/// Sum block_reward_at(e) for e in 1..first_sealed_epoch. Era- and cap-aware.
/// Call once at cold start with the consensus-agreed first sealed epoch.
pub fn genesis_gap_recycle_hunits(first_sealed_epoch: u64) -> u64
```

Unit-tested (`genesis_gap_recycle_basic`, `_matches_block_reward_sum`, `_plus_realized_preserves_cap`); `total_supply_correct` unchanged. For `F = 66_571` it returns `133_140 HONE`.

## Wiring to fold into the BUG 6 fix (`src/main.rs`, reward driver cold-start block ~438-479)

At the point where the cold-start path establishes the consensus-agreed first finalized epoch `first_final` (== global `F`, since no epoch below it is in the agreed sealed set), credit the recycle fund **once**, before/at the `last_rewarded = first_final - 1` skip:

```rust
// One-time genesis-gap recycle (unearned pre-launch rewards -> end-of-chain pool).
// first_final is derived from the AGREED EpochFinalize set, so every node computes
// the same F and the same amount -> no state_root divergence. Fires once; the
// durable key makes it idempotent across restart and (with state-sync) late-join.
const GENESIS_GAP_KEY: &str = "genesis_gap_recycled";
if chain_ref.store.state_get(GENESIS_GAP_KEY).is_none() {
    let amount = hone_types::genesis_gap_recycle_hunits(first_final);
    if amount > 0 {
        let _ = chain_ref.store.credit(
            hone_types::RECYCLE_FUND_ACCOUNT, hone_types::NATIVE_TOKEN, amount);
        info!("[finalize] genesis gap epochs 1..{} unearned — {} hunits -> recycle fund",
              first_final, amount);
    }
    let _ = chain_ref.store.state_set(GENESIS_GAP_KEY, b"1");
}
```

### Determinism requirements (the BUG 6 core)
- `first_final` MUST be the quorum-agreed first sealed epoch (from the `EpochFinalize`/`sealed_by` set), never a local `has_block`/wall-clock value. This is exactly the "derive unsealed from the agreed set" rule BUG 6 mandates; the gap-credit is just its `1..F` extension.
- Keep the credit inside the same `REWARD_FINALITY_DEPTH`-gated, replay-derived path as the normal per-epoch recycle so all nodes apply it at the same agreed point.
- `genesis_gap_recycled` must be a durable RocksDB key (like `epoch_finalized_done:{e}`) so restart doesn't re-credit; `highest_contiguous_rewarded` reconstruction must not bypass it.
- Late-join: a node that state-syncs after `F` inherits both the credited balance and the key — no re-credit. Same "not late-join-safe without raw replay" caveat that already applies to all recycle; state-sync is the intended path.

### Must not disturb
- Live empty-sealer recycle (`main.rs` empty-sealer branch) and `emit_epoch_rewards` splits — unchanged; the gap-credit composes with them into one universal rule.
- Wallet-liveness entropy decay -> recycle (`finalize::apply_entropy_decay`): same pool, separate source; untouched. Backdated launch does not prematurely decay genesis accounts (silence at launch ~=66.5k epochs << `LIVENESS_GRACE_EPOCHS` ~=3.15M).

## Verification (run against the combined BUG 6 + gap-recycle result)
1. `cargo +1.92.0 test -p hone-types` — helper tests + `total_supply_correct` (green already).
2. New node test: an empty-sealer finalized epoch credits `__recycle_fund__` (currently untested; `finalize.rs` has no tests).
3. **2-clock isolated dry-run (the BUG 6 repro).** Boot two clocks in `HONE_ISOLATED` against the real backdated `genesis.json`; seal past `F + REWARD_FINALITY_DEPTH`; assert both nodes' `state_root` AND `__recycle_fund__` balance match, and recycle ~= `genesis_gap_recycle_hunits(F)` plus any live empty epochs. Convergence here is the pass/fail gate for a multi-clock launch.

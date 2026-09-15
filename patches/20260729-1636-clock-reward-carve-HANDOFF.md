---
id: c3f8a1d29b4e
from: shindevlin
to: natoshisakamoto
type: fix-handoff
created: 2026-07-29T16:36:00Z
in_reply_to: null
reply_expected: true
---

# Fix: clock reward is minted OUTSIDE the epoch budget (42M cap violation)

## Bug
In `rust/hone-node/src/main.rs` `emit_epoch_rewards`, the clock reward is minted **additively**: `base_clock_reward = clock_reward_at(epoch)` is paid PER SEALER via `apply_entry(ClockReward)` -> `distribute_role_backer_reward`, with **no debit from `raw_pool` / `block_reward_at`**. So realized emission each epoch = `block_reward_at` + `clock_reward_at x sealers` (+ testnet, within budget). That drifts realized supply **above the 42,000,000 HONE hard cap**.

This violates whitepaper 1.2 Layer D — clock node rewards are part of the base, "well under 1% of the block reward", i.e. **carved from** the block reward — and the closing rule: "Total emission each epoch equals base plus activity plus fee boost, hard-capped at the Layer A adjusted ceiling. Any remainder flows to hone_recycle." And Shin's explicit directive: the epoch has ONE fixed emission; pay what's earned, recycle the rest, never mint on top ("never emit just for emitting").

Audit note: the nine activity pools, the 2% reserve, and TestnetReward (paid from the fund, fund-capped) are all already within budget. ClockReward is the ONLY live additive mint. `RuntimeReward` has no live emission site; the `inference_daemon::distribute_clock_rewards` fn is dead (`#[allow(dead_code)]`).

## Fix (applied — diff below)
Carve the clock allotment from `raw_pool` before `activity_pool`, and recycle the unearned remainder so the clock line is conserved (paid-or-recycled), never minted beyond the ceiling:
- `clock_allotment = base_clock_reward * clock_sealers.len()`, carved from post-reserve budget (`activity_pool = post_reserve - clock_allotment`).
- Accumulate `clock_paid` (actually-paid, uptime-scaled); `clock_recycle = clock_allotment - clock_paid` -> `RECYCLE_FUND_ACCOUNT`.
- Basis = **SEALERS**, not registered/staked nodes: bootstrapping nodes hold zero stake until they earn it (stake-gated allotment would pay nobody at launch, chain can't bootstrap), and sealer-basis stops idle/fake registrations from inflating the carve. Confirmed with Shin.

## Asks
1. Apply to the **launch branch** (the branch you build the launch binary from — it MUST already contain the BUG6 reclaim `efd98c6e2`). Heads-up: the desktop working tree is currently on `feat/model-router`; the BUG6 reclaim landed on `fix/finalizer-determinism`. Please confirm which is canonical for launch and make sure BOTH fixes are on it. Commit author **Shin Devlin <shindevlin@proton.me>**, no AI attribution, branch-only, never main.
2. `cargo +1.92.0 check -p hone-node`, then build all three node binaries.
3. Run the isolated 3-clock gate. In addition to the existing `state_root` + `__recycle_fund__` convergence, assert **SUPPLY CONSERVATION**: for each finalized epoch (era < 5), total newly-credited hunits (all reward payouts + all recycle credits) == `block_reward_at(epoch)` EXACTLY — i.e. per-epoch minted-vs-ceiling delta == 0, no clock mint beyond the ceiling. Add the assertion if not already present.
4. Do NOT flip. Clocks stay off until Shin enables them in person.

Report back: compile result, 3-clock convergence result, and the measured per-epoch minted-vs-ceiling delta (expect 0). Patch also saved in the repo at `patches/clock_reward_carve.patch`.

## Diff
```diff
diff --git a/rust/hone-node/src/main.rs b/rust/hone-node/src/main.rs
index 65b0c136f..f5d32abb9 100644
--- a/rust/hone-node/src/main.rs
+++ b/rust/hone-node/src/main.rs
@@ -1884,7 +1884,25 @@ fn emit_epoch_rewards(
     let recycle_split  = reserve_total
         .saturating_sub(testnet_top_up)
         .saturating_sub(treasury_split);                                  // 1.5%
-    let activity_pool  = raw_pool.saturating_sub(reserve_total);
+    // ── Layer D clock base: carved from the epoch budget, NOT minted on top ──
+    // The clock reward is part of the capped block-reward emission (whitepaper
+    // §1.2 Layer D), never an additive mint. Each SEALING clock node has a fixed
+    // base_clock_reward allotment this epoch; the total is carved out of the budget
+    // here so total emission can never exceed block_reward_at(epoch). Sealers earn
+    // their allotment scaled by uptime; the unearned remainder recycles below.
+    // Sized on sealers (not merely registered/staked nodes): bootstrapping nodes
+    // hold zero stake until they earn it, so a stake-gated allotment would pay no
+    // one at launch; gating on actual seals also stops idle/fake registrations from
+    // inflating the carve.
+    let base_clock_reward = clock_reward_at(epoch);
+    let clock_allotment = base_clock_reward.saturating_mul(clock_sealers.len() as u64);
+    let post_reserve = raw_pool.saturating_sub(reserve_total);
+    if clock_allotment > post_reserve {
+        warn!("clock: epoch {} clock_allotment {} exceeds post-reserve budget {} — \
+activity pool starved (sealing clock-node count too high for era budget)",
+            epoch, clock_allotment, post_reserve);
+    }
+    let activity_pool  = post_reserve.saturating_sub(clock_allotment);
 
     if recycle_split > 0 {
         let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_split);
@@ -1903,8 +1921,6 @@ fn emit_epoch_rewards(
     const CLOCK_UPTIME_WINDOW: u64 = 100;
     const CLOCK_UPTIME_MIN_EPOCHS: u64 = 10;
 
-    let base_clock_reward = clock_reward_at(epoch);
-
     // Collect all registered clock nodes for the uptime window update.
     let registered_nodes: Vec<String> = chain.store.state_scan_prefix("clock_reg:")
         .into_iter()
@@ -1942,7 +1958,9 @@ fn emit_epoch_rewards(
         ).unwrap_or_default());
     }
 
-    // Emit ClockReward for each sealer, scaled by uptime.
+    // Emit ClockReward for each sealer, scaled by uptime. Whatever of the carved
+    // clock allotment is not earned here recycles below (paid-or-recycled).
+    let mut clock_paid: u64 = 0;
     for node_id in clock_sealers {
         if base_clock_reward == 0 { break; }
         let uptime_key = format!("clock_uptime:{}", node_id);
@@ -1963,9 +1981,17 @@ fn emit_epoch_rewards(
             warn!("clock: clock reward failed for {} epoch {}: {}", node_id, epoch, e);
             continue;
         }
+        clock_paid = clock_paid.saturating_add(scaled_reward);
         // No broadcast: rewards are replay-derived from the winning EpochFinalize on
         // every node; gossiping ClockReward would double-credit (no idempotency guard).
     }
+    // Unearned clock allotment (uptime shortfall, skipped/failed payouts, or any
+    // budget-capped remainder) recycles — the clock line is conserved and never
+    // minted beyond the epoch budget carved above.
+    let clock_recycle = clock_allotment.saturating_sub(clock_paid);
+    if clock_recycle > 0 {
+        let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, clock_recycle);
+    }
 
     // ── Layer E: testnet operator rewards (from testnet fund) ────────────────
     let testnet_ops: Vec<(String, String)> = chain.store.state_scan_prefix("testnet_op:")
```

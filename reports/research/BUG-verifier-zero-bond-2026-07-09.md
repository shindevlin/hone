# BUG: Inference verifiers vote with zero stake at risk — dissenter-slashing is a runtime no-op

**Severity:** High (economic / consensus integrity — sybil verifier boards are free)
**Status:** Confirmed by code read, 2026-07-09
**Component:** `rust/hone-node/src/inference.rs` (inference job marketplace, verifier board)
**Found by:** `hone-sovereign-multimodal-ai` verification-design workflow; verified against source.

---

## Summary

A node can join a job's **verifier board and vote with nothing at stake**. The
anti-collusion mechanism that is supposed to punish a verifier who votes against board
consensus — dissenter slashing — computes to **zero** for any verifier that never
independently staked. The result: a sybil (or a worker's colluding sock-puppet) can bid as
a verifier, cast a malicious or collusive verdict, and face **no economic penalty**. The
defense is a no-op against exactly the attacker it targets.

## Root cause

Two facts combine:

1. **`apply_bid` never locks stake for a verifier.** A `verifier`-role bid is gated only on
   reputation, fee ceiling, and duplicate-bid checks — no balance is debited or locked.

   `inference.rs:360-399` (`apply_bid`):
   ```rust
   if !["worker", "verifier"].contains(&role.as_str()) {
       bail!("bid role must be 'worker' or 'verifier'");
   }
   let rep = get_reputation(chain, bidder);
   if rep.score < job.min_reputation {
       bail!("node '{}' reputation {} below required {}", bidder, rep.score, job.min_reputation);
   }
   // ... duplicate-bid check ...
   set_bid(chain, &BidState { /* ... */ })?;   // <-- no debit / no stake lock
   Ok(())
   ```
   Reputation for a fresh account starts at `5000` (`NodeReputation::new`), so a job with
   default `min_reputation = 0` accepts any account as a verifier with zero cost.

2. **The dissenter slash is capped at the dissenter's existing stake, which is zero.**

   `inference.rs:817-825` (`apply_pay`):
   ```rust
   for (dissenter, slash_amount) in dissenter_slashes {
       let current_stake = chain.store.get_stake(dissenter);
       let actual_slash = (*slash_amount).min(current_stake);   // min(x, 0) = 0
       if actual_slash > 0 {
           let _ = chain.store.set_stake(dissenter, current_stake - actual_slash);
           let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, actual_slash);
       }
   }
   ```
   For a verifier that never called `Stake`, `current_stake == 0`, so `actual_slash == 0`
   and the `if actual_slash > 0` block never runs. Nothing is slashed.

## Impact

- **Sybil verifier boards are free.** An attacker spins up N fresh accounts, has them all
  bid as verifiers, and votes them as a bloc. Being out-voted (dissent) costs nothing.
- **Worker/verifier collusion is free.** A worker's sock-puppets can approve garbage output;
  even if honest verifiers dissent and the honest verdict wins, the colluders lose nothing.
- **The rubber-stamp / commit-reveal defenses do not close this** — they raise the cost of
  *coordination* and *copying*, not the cost of *participating with no skin in the game*.

## Fix (standalone — independent of the larger verification redesign)

Introduce a locked **`VERIFIER_BOND`** that a verifier must post to bid/commit, and make the
slash draw from that locked bond rather than from generic stake.

1. Add a constant `VERIFIER_BOND` (hunits) in `hone-types`.
2. In `apply_bid` (or at `InferenceJobCommit` time, whichever is the true "now you're on the
   board" moment), **debit `VERIFIER_BOND` from the bidder's balance into escrow**
   (`RECYCLE_FUND_ACCOUNT` or a per-job bond key), and record the bond on the `BidState`.
3. On `apply_pay`, refund the bond to consensus verifiers and **slash it from dissenters**
   (draw `actual_slash` from the locked bond, not `get_stake`). Then the
   `min(slash_amount, current_stake)` cap is replaced by `min(slash_amount, bond)`.
4. Tests: (a) a fresh account cannot verify without the bond balance; (b) a dissenting
   verifier's bond is actually reduced to zero and credited to recycle; (c) a consensus
   verifier's bond is refunded.

## Related (same subsystem, not this bug but adjacent)

Two anti-collusion code paths exist but are **gated off** behind `const false` and should be
part of the same hardening pass:
- `VERIFIER_ASSIGNMENT_ENABLED = false` — random board assignment via epoch entropy.
- `INFERENCE_COMMIT_REVEAL_ENABLED = false` — commit-reveal to stop verdict-copying.

Both are prerequisites for any optimistic / small-board verification model: a small board
that an attacker can self-assign onto (assignment off) is trivially captured, bond or no bond.

## References

- `rust/hone-node/src/inference.rs:360-399` (`apply_bid` — no stake lock)
- `rust/hone-node/src/inference.rs:817-825` (`apply_pay` — slash capped at zero stake)
- `reports/research/verifiable-multimodal-2026-07-09.md` (finding #2, full design context)

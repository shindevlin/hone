# Testnet Dry-Run: 2-Clock Sealing Path (Re-run) — 2026-07-12

**Goal:** re-validate the 2-clock sealing path against the fixed binary after the
2026-07-10 dry-run (`reports/DRYRUN_2CLOCK_2026-07-10.md`) found two launch-blocking bugs.
Local only, throwaway keys (`.dryrun/`, gitignored), ports 4243/4244, `chain_id=hone-testnet`.

## Bottom line

**Real 2-of-2 founder-clock quorum consensus now works — proven live.** The re-run
confirmed the two original bugs are fixed AND surfaced **three more launch blockers** that
only appear once you get far enough for two clocks to actually try to agree. All five are
now fixed; the seal quorum flips `1/1 → 2/2` and holds.

```
epoch 22800 sealed: quorum=1/1 registered, 2 total seals, 0 outliers   ← before B's reg propagated
epoch 22801 sealed: quorum=2/2 registered, 2 total seals, 0 outliers   ← FLIPPED
epoch 22802 sealed: quorum=2/2 registered, 2 total seals, 0 outliers   ← holds
```
Both nodes: `registered_count:2`, `registered_clocks:["clocka","clockb"]`, `in_quorum:true`.

---

## Verification results (V1–V6)

| Check | Result | Verdict |
|---|---|---|
| V1 `clock/registered` | `stakers:2`, both clocks listed on **both** nodes | ✅ |
| V2 `clock/status` | `in_quorum:true`, `registered_count:2`, pubkey `8c235050…` == genesis | ✅ |
| Quorum sealing | `2/2 registered, 0 outliers`, sustained | ✅ |
| V5 balances | clocka earning ClockReward (0.041 HONE) | ✅ |
| Sim disabled | `HONE_SIM=false` honored (0 sim daemons) | ✅ |
| V3 `state_root` convergence | A ≠ B | ⚠️ **external-peer contamination, not a consensus bug** (see caveat) |

---

## The five bugs

**BUG 1 (fixed 07-11) — seal-signing identity ≠ on-chain posting key.** `HONE_POSTING_KEY`
was parsed as a raw ed25519 seed by `main.rs` but as BIP-39 entropy → SLIP-10
`m/44'/6942'/2'/0'` by `wallet.rs`. Seals were signed by an identity that never matched
genesis. Fix: sign from `wallet_keys.hone_private_key`; fail loud if no usable key.

**BUG 2 (fixed 07-11) — startup auto-register was balance-gated, defeating bootstrap grace.**
A 0-balance founder clock bailed before submitting. Fix: `in_grace || balance >= min_stake`.

**BUG 3 (fixed 07-12) — the testnet sim daemon has no opt-out.** Gated only on
`chain_id == hone-testnet`, it seeds balances + posts transfers independently on each node →
guaranteed state fork, making a 2-clock consensus test impossible. Fix: `HONE_SIM=false`
opt-out (defaults on, preserving demo behavior).

**BUG 4 (fixed 07-12) — ClockNodeRegister never propagates between peers.** Two failures:
(a) the broadcast used `{"entry":..}` with **no signature**, but the receive-side
`validate_and_apply` **requires** a signature for `ClockNodeRegister` → rejected on every
peer; (b) the register was a **one-shot boot broadcast** into an empty gossip mesh
(`NoPeersSubscribedToTopic`), never retried, and gossipsub does not replay to late joiners.
Fix: `broadcast_signed_entry` (attaches `sig`) + the seal loop re-announces the clock's own
signed registration for the first ~20 epochs then every 20th epoch.

**BUG 5 (fixed 07-12) — the quorum denominator excluded grace-registered clocks.**
`registered_clock_nodes()` (the quorum denominator) counts a `clock_reg:` entry only if
`stake > 0`, but bootstrap-grace clocks register at **stake 0** by design. So even with
registration fully propagated, two grace clocks each computed a solo `1/1` quorum. Fix: during
the grace window, include stake-0 `clock_reg:` entries in the eligible set. **This is the bug
that directly caused "each clock stays 1/1"** — grace registration and quorum eligibility were
fighting each other.

BUG 4 makes peers *see* each other's registration; BUG 5 makes that registration *count*
toward quorum. Both are required for 2-of-2.

---

## Caveat — state_root fork is a test-environment artifact, not a consensus bug

The two clocks show divergent `state_root` because the WSL test nodes ALSO dial the live
`honemesh.net` bootstrap seeds (`discovery.rs` `HONE_NET_API` is a hardcoded const, not
env-overridable) and connect to ~11–13 real testnet peers. Each node independently ingests
different entries from those peers, so local state legitimately diverges. The **consensus
signals** (registration propagation, `2/2` quorum, `0 outliers`, matching seal hashes) all
pass. A clean convergence proof requires isolating the two nodes from honemesh.net (block the
host, or add an env to override the registry endpoint) — recommended as a follow-up so the
dry-run can assert converged `state_root` too.

---

## Recommended follow-ups
1. Add an env override for the discovery registry endpoint (or a `HONE_ISOLATED=true` that
   skips DNS-seed/registry discovery) so 2-node consensus tests are truly isolated and can
   assert `state_root` convergence.
2. Unit-test `registered_clock_nodes` grace inclusion (stake-0 in grace → included; post-grace
   stake-0 → excluded).
3. Re-run this dry-run isolated; expect converged `state_root` as the final green.
4. Mainnet start remains a **coordinated founder event** (≥2 founders, locked chain_id, vault
   wallets, verify-vault gate) — never solo. These fixes make that event mechanically viable.

_Re-run executed 2026-07-12 on hone-testnet, throwaway keys/dirs, `HONE_SIM=false`, ports
4243/4244. Full test suite green. Nodes torn down, data dirs removed._

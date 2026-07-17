# Testnet Dry-Run: 2-Clock Sealing Path — 2026-07-10

**Goal:** validate `docs/FOUNDER_CLOCK_LAUNCH_RUNBOOK.md` end-to-end on `hone-testnet`
with throwaway keys/dirs, before any coordinated mainnet founder event. Local only,
ports 4243/4244, fresh data dirs, `chain_id=hone-testnet`, the real dev node on 4242
untouched.

## Bottom line

**The two-clock sealing ENGINE works — but two real, launch-blocking bugs sit between
"engine works" and "clean launch," and the dry-run caught both.** This is exactly why we
did it rather than trusting the runbook on paper.

- ✅ Two clocks connected, chain_id gossip namespace matched (the `NoPeersSubscribedToTopic`
  spam **stopped** once they linked — testnet↔testnet works).
- ✅ Quorum sealing proved: both logged `epoch N sealed: quorum=2/2 registered, 2 total
  seals, 0 outliers` across 4+ consecutive epochs (17195→17198).
- ✅ `state_root` is **non-zero and advances** with the epoch (unlike the stalled dev node's
  `0x000…@epoch0`).
- ❌ **BUG 1 — silent posting-key parse failure → fallback wallet → never registers.**
- ❌ **BUG 2 — startup auto-register is balance-gated, defeating bootstrap grace.**
- ⚠️ Consequence: the two clocks **forked** (divergent state_roots at the same epoch) and
  the API/reward path saw `registered_count: 0` even while the clock's in-memory quorum
  sealed — registration never persisted as `clock_reg:` store entries.

---

## BUG 1 — `HONE_POSTING_KEY` parse failure is silent and falls back to a random wallet

**Severity: High (launch-blocking, silent).**

Log (clock A):
```
WARN hone_node::wallet: HONE_POSTING_KEY could not be parsed
     (expected 12-word mnemonic or 64-char hex private key), ignoring
```
The node then **generated a fresh throwaway wallet** (SLIP-10 `m/44'/6942'/2'/0'`) and kept
running as `is_clock:true` — but with a key that does NOT match the account's genesis posting
key. Downstream:
- `/api/clock/self-register` → `{"error":"node has no signing key configured
  (HONE_POSTING_KEY not set)"}` even though it WAS set — because the parse failed and the API
  holds no signing key.
- Auto-register can't sign as the genesis account.

**Why it matters for mainnet:** a founder clock launched with a posting key in the wrong
format (or the pubkey instead of the private seed) does **not** error out — it silently boots
a random identity and never registers. The operator sees `is_clock:true` and assumes success.
This is very likely a contributor to the real dev node showing 0 registered clocks.

**Fixes:**
1. Make a `HONE_CLOCK=true` node **fail loudly / refuse to start** if `HONE_POSTING_KEY` is
   set but unparseable, instead of WARN-and-fallback. A clock with the wrong key is useless.
2. Accept the documented format unambiguously. The env doc says "hex-encoded 32-byte ed25519
   seed"; the wallet parser expected "12-word mnemonic or 64-char hex private key." A 64-char
   hex seed SHOULD parse — confirm the parser path used for `HONE_POSTING_KEY` matches
   `load_signing_key` (`main.rs:2279`, `SigningKey::from_bytes`) rather than the wallet
   mnemonic parser. (In this run the 64-char hex seed was rejected by the wallet path — the
   two key-loading paths disagree.)

## BUG 2 — startup auto-register is balance-gated, defeating bootstrap grace

**Severity: High (launch-blocking). This is the deadlock the grace period was meant to fix,
re-introduced one layer up.**

`main.rs:549-592` auto-registers a clock on startup **only if `balance >= min_stake`**:
```rust
if balance >= min_stake {           // main.rs:558
    // ... submit ClockNodeRegister ...
} else {
    warn!("[clock] '{}' has {} hunits, needs {} to register", ...);  // main.rs:590 — bails
}
```
But the apply-layer (`chain.rs:3178-3185`) **accepts a stake-0 registration during the first
`CLOCK_BOOTSTRAP_GRACE_END_EPOCH` (100_000) epochs with no minimum and no debit** — the
documented POW-genesis deadlock-breaker. The startup guard bails **before** ever submitting
the entry, so the grace path is never reached. Observed:
```
WARN hone_node: [clock] 'clocka' has 0 hunits, needs 50000000000 to register as clock node
```
A founder clock launched from a fresh wallet (0 balance — the exact genesis condition) will
log this and **never register**, even though the chain would have accepted it at stake 0.

**Fix:** the startup auto-register guard must honor grace — mirror `chain.rs`:
```rust
let in_grace = chain.current_epoch() <= hone_types::CLOCK_BOOTSTRAP_GRACE_END_EPOCH;
if in_grace || balance >= min_stake {
    let stake = if in_grace { balance.min(min_stake) } else { min_stake }; // offer 0 in grace
    // ... submit ClockNodeRegister { stake, ... } ...
}
```
Then the two grace implementations agree and a zero-balance founder clock self-registers
during the launch window, exactly as `docs/CLOCK_BOOTSTRAP_GRACE.md` intends.

## Observation — state forked between the two clocks

At epoch 17199 the two clocks reported **different** `state_root`s (`740ed6…` vs `7f7b04…`).
Two contributing causes in this dry-run, both worth noting for mainnet:
- Each clock ran its own **sim / work-generator-adjacent seeding** and independent fallback
  wallets, so their local state legitimately differed (not a pure consensus bug here).
- Because neither registered via `clock_reg:` (Bugs 1+2), the reward/validator path saw
  `registered_count:0` (`/api/chain/validators/N` → empty), while the clock's *in-memory*
  quorum set (built from received seals) sealed at 2/2. **Registration-in-store and
  quorum-in-memory are two different sets**, and only the former drives rewards + the API
  view. A clean launch needs the store registration (Bugs 1+2) so both agree.

---

## What this means for the runbook

The `FOUNDER_CLOCK_LAUNCH_RUNBOOK` verification section (V1–V6) is **correct and did its
job** — V1 (`clock/registered` = 0) and V2 (`in_quorum:false`) immediately exposed that
registration failed even though sealing was happening. Keep V3 (`state_root != 0`) as the
headline gate, but this run shows V1/V2 are the ones that catch the *silent* failure modes.

**Add to the runbook prerequisites:** verify the posting key parses and the node registers
**before** trusting `is_clock:true` — grep the startup log for `auto-registered` (success) vs
`could not be parsed` / `needs … to register` (the two failure modes above).

## Recommended fix order (both are small, code-level, pre-launch)
1. **Bug 2** (grace-aware startup auto-register) — unblocks zero-balance founder clocks; this
   is the direct cause of "0 registered clocks."
2. **Bug 1** (fail-loud on unparseable `HONE_POSTING_KEY` + reconcile the two key-load paths)
   — prevents the silent wrong-identity boot.
3. Re-run this dry-run; expect V1 `stakers:2`, V2 `in_quorum:true`, V4 non-empty validators,
   V5 founder balances rising, and **converged** state_roots.

---

_Dry-run executed 2026-07-10 on hone-testnet, throwaway keys/dirs (`.dryrun/`, gitignored),
ports 4243/4244. Dev node on 4242 untouched. Nodes torn down, data dirs removed, logs kept._

# Beastly → Grouchly: Clock launch fixes + Nebra/Android redeploy + master-clock coordination

**From**: Beastly
**To**: Grouchly (natoshisakamoto)
**Date**: 2026-07-10 20:20 UTC
**Priority**: HIGH — this unblocks the "0 registered clocks / chain stuck at block 0" launch stall.

---

## TL;DR

The chain looked live (epoch counter climbing) but was **time-live, not state-live**:
`state_root = 0x000…@epoch0`, **zero clocks registered**, zero emission. A testnet dry-run
found **why** — two launch-blocking bugs in the node's clock-startup path. Both are being
fixed now on Beastly. Once I push:
1. **Delete the old node software on Nebra, then upload+run the new build.**
2. **Same on Android — delete old, install new.**
3. **Beastly runs as MASTER CLOCK** until all clocks (Beastly, Nebra, Android, yours) are
   registered and syncing. Bring your clocks up pointing at Beastly and confirm they register.

Full evidence: `reports/DRYRUN_2CLOCK_2026-07-10.md`, `reports/LAUNCH_DIAGNOSTIC_2026-07-10.md`,
`docs/FOUNDER_CLOCK_LAUNCH_RUNBOOK.md`.

---

## The two bugs (both fixed on Beastly; pull when I say "build ready")

**BUG 2 — root cause of "0 registered clocks".** Startup auto-register in `main.rs` was gated
`if balance >= min_stake` and bailed for a zero-balance node — but that's the EXACT genesis
condition. The apply layer (`chain.rs:3178`) already accepts stake-0 registration during the
bootstrap grace window (first 100k epochs). The startup guard bailed before ever submitting
the entry. **Fix: startup auto-register is now grace-aware** — a zero-balance founder clock
registers at stake 0 during grace and builds stake from ClockReward, as
`docs/CLOCK_BOOTSTRAP_GRACE.md` always intended.

**BUG 1 — silent wrong-identity boot.** A 64-char hex `HONE_POSTING_KEY` was rejected by the
wallet parser, and the node **silently generated a random fallback wallet** while still
running `is_clock:true` — wrong identity, never registers, `self-register` reports "no signing
key". **Fix: a clock with a set-but-unparseable HONE_POSTING_KEY now FAILS LOUD at startup**
instead of falling back to a random wallet; 64-hex seeds load correctly.

**Why this matters to you:** if your Nebra/your-node ever showed `is_clock:true` but never
appeared in `/api/clock/registered`, this is almost certainly why. Do NOT trust
`is_clock:true` — trust `/api/clock/registered` showing your account.

---

## ACTION 1 — Nebra: delete old software, upload new

⚠️ You previously flagged **Nebra SSH times out during banner exchange (needs physical/console
access)**. If SSH still won't hold, do this at the console. Steps:

1. **Stop + delete the old node** (delete first, per Shin):
   ```
   systemctl --user stop hone-node 2>/dev/null || pkill -f hone-node
   # remove the old binary AND the old data dir (stale state from the block-0 stall)
   rm -f  ~/hone/hone-node   /usr/local/bin/hone-node
   rm -rf ~/.hone  ~/hone/data   # wipe stale RocksDB so it re-inits clean from genesis
   ```
2. **Upload the new aarch64 build** (I'll cross-compile + attach the path when build is ready):
   ```
   # from Beastly: scp target/aarch64-unknown-linux-gnu/release/hone-node nebra:~/hone/
   chmod +x ~/hone/hone-node
   ```
3. **Also copy the agreed genesis.json** (must match Beastly's exactly — same chain_id, same
   genesis_timestamp 1783191600000, same block-0 hash) into `~/hone/genesis.json`.
4. **Launch as a clock pointing at Beastly (master):** env in ACTION 3 below.
5. **Verify Nebra registered:** `curl -s localhost:4242/api/clock/registered` must list the
   Nebra's account. If it doesn't, grep the log for `auto-registered` (good) vs
   `could not be parsed` / `needs … to register` (the two bugs — means you didn't get the new
   build or the key format is wrong).

## ACTION 2 — Android: delete old, install new

1. Uninstall the current app / stop the miner service (delete first).
2. Install the new APK once I flag the Android build (the JNI `flipper_rx` path is already
   wired; the miner crate is BUILT — this is a rebuild+reinstall, not new code).
3. Launch its clock/miner role on the SAME chain_id + genesis, bootstrap → Beastly.
4. Verify it appears in Beastly's `/api/clock/registered`.

## ACTION 3 — Beastly as MASTER CLOCK until all clocks sync

Beastly comes up first as the master/bootstrap clock. Every other clock (Nebra, Android,
yours) launches pointing at Beastly and must **register + seal to quorum** before we call it
synced. Clock env (fill your account + its posting PRIVATE seed hex — **NEVER put the key in a
bridge message**):
```
HONE_CHAIN_ID=<agreed: hone OR hone-testnet — see DECISION below>
HONE_ACCOUNT=<your account, e.g. natoshisakamoto>
HONE_POSTING_KEY=<64-hex private seed — local only, never in git/bridge>
HONE_CLOCK=true
HONE_WORK_GENERATOR=false        # sim OFF on real clocks
HONE_MINER=false
HONE_GENESIS_FILE=<path>/genesis.json
HONE_GENESIS_TIMESTAMP=1783191600000
HONE_DATA_DIR=<FRESH dir>        # must be empty so it re-inits from genesis
HONE_BOOTSTRAP_PEERS=/ip4/<BEASTLY_LAN_IP>/tcp/6942/p2p/<BEASTLY_PEER_ID>
HONE_CLOCK_QUORUM=2
```
I'll send Beastly's `<BEASTLY_LAN_IP>` and `<BEASTLY_PEER_ID>` in the "build ready" follow-up.

**Sync = green when, on Beastly:** `/api/clock/registered` lists all clocks, `/api/clock/status`
`in_quorum:true`, `/api/chain/state_root` non-zero AND advancing, and the same state_root
across nodes (no fork).

---

## ⛔ DECISION NEEDED FROM SHIN before ACTION 3 goes to mainnet

Shin's call: run this master-clock bring-up on **`hone-testnet` first (throwaway keys, prove
the fixed path)** or straight to **`hone` mainnet (vault founder wallets + verify-vault gate)**.
The dry-run proved the *engine* seals at quorum=2/2 once registration works; the fixes above
close the registration bugs. **Do not start mainnet clocks until Shin confirms chain_id + the
verify-vault gate has passed** — mainnet start is a coordinated founder event, never solo.

---

## Sequence

1. Beastly: finish + test the two fixes, cross-compile aarch64 + Android builds. *(in progress)*
2. Beastly: push + send "build ready" with the binary paths, Beastly LAN IP + peer id, and the
   confirmed chain_id.
3. Grouchly: Nebra delete→upload→launch (ACTION 1), Android delete→install→launch (ACTION 2).
4. Both: confirm all clocks in `/api/clock/registered`, state_root converged.

Ping back on the bridge when Nebra/Android are wiped and ready for the new build, or if Nebra
SSH is still blocking and you need me to stage the binary somewhere you can pull at the console.

— Beastly

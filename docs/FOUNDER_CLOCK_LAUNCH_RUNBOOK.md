---
title: HONE Founder-Clock Launch Runbook — bringing the chain from time-live to state-live
description: The operational steps to get ≥2 founder clocks sealing real epochs, and to VERIFY state actually advances (not just the wall-clock epoch counter)
author: Shin Devlin
---

# Founder-Clock Launch Runbook

**Purpose:** the genesis runbook (`docs/GENESIS_LAUNCH_RUNBOOK.md`) covers wallet
recoverability, genesis.json integrity, and starting two nodes. This runbook covers the
**next, separately-failing layer**: getting founder **clocks** to actually **seal epochs so
chain state advances past block 0**, and — critically — **verifying that it did.**

> **Why this exists (the 2026-07-10 lesson).** On that date the running node showed
> `chain_id: hone, epoch: 17000+, peers: 21` and looked live. It was NOT: `state_root =
> 0x000…000 @ epoch 0`, **zero clocks registered, zero blocks sealed, zero emission.** The
> epoch counter climbs from wall-clock (`(now − genesis_ts)/30s`) whether or not anyone
> seals. **A climbing epoch number is NOT proof of a live chain.** See
> `reports/LAUNCH_DIAGNOSTIC_2026-07-10.md`. This runbook's verification steps exist so that
> false-positive can never be mistaken for launch again.

---

## The one rule (clock layer)
> **The chain is state-live only when `state_root != 0` AND ≥2 clocks are registered AND
> founder balances are rising from ClockReward.** Nothing less counts. Not the epoch number.

---

## Prerequisites (must be TRUE before starting a clock)

- [ ] Genesis runbook completed through **step 10** — `verify-vault` passed, both nodes
      reproduce the **same block-0 hash** (`98e3c1b0…` for the re-genesis), vault backed up
      off-machine.
- [ ] **chain_id is LOCKED and identical everywhere.** Decide `hone` (mainnet) vs
      `hone-testnet` ONCE. Every clock, the genesis file, and the discovery bootstrap must
      use the same value. (The 2026-07-10 stall was partly a `hone` vs `hone-testnet` drift:
      gossip topics are `{chain_id}/hone/entries`, so a mismatched node publishes to a topic
      no peer is subscribed to → `NoPeersSubscribedToTopic` forever.)
- [ ] The founder wallets exist as **keystores in the vault** (not boot-generated throwaway
      mnemonics). You will pass each clock its account's **posting PRIVATE key seed**.

---

## Config reference — the env vars that actually matter (from `main.rs`)

| Var | Value for a founder clock | Notes |
|---|---|---|
| `HONE_CHAIN_ID` | the LOCKED value (`hone` or `hone-testnet`) | MUST match on every node |
| `HONE_ACCOUNT` | the founder account name (e.g. `natoshisakamoto`) | not `genesis` |
| `HONE_POSTING_KEY` | hex 32-byte ed25519 **seed** (private) | node_id derives from it; NOT the pubkey |
| `HONE_CLOCK` | `true` | **the gate that makes the node seal** (default off) |
| `HONE_GENESIS_FILE` | path to the agreed `genesis.json` | same file on every node |
| `HONE_GENESIS_TIMESTAMP` | `1783191600000` | must match all nodes |
| `HONE_DATA_DIR` | a FRESH dir per clock | node refuses to re-init over an existing block 0 |
| `HONE_BOOTSTRAP_PEERS` | the other clock's multiaddr | so they find each other |
| `HONE_CLOCK_QUORUM` | leave default `2` | min unique sealers to seal an epoch |
| `HONE_WORK_GENERATOR` | **unset / false** | `true` = the SIM (fake jobs). OFF on production. |
| `HONE_MINER` | `false` on a pure clock | mining is a separate role |

**Quorum math (`clock.rs`):** an epoch seals when `≥ max(HONE_CLOCK_QUORUM,
ceil(registered × 0.51))` unique registered clocks agree on the seal hash. With 2 registered
clocks, both must seal. **One clock alone cannot advance a multi-clock chain** (it can
self-seal only in the isolated/bootstrap-single-clock branch). → **Launch two from the
start.**

---

## Launch sequence

### 1. Confirm the sim is OFF and no stale node is running
```
# kill any dev/sim node still holding port 4242
# (the 2026-07-10 node ran HONE_WORK_GENERATOR + account=genesis on testnet)
curl -s http://localhost:4242/api/node/info   # if this answers, a node is up — stop it first
```
A production clock log must NOT contain `sim tick … (epoch 0)`.

### 2. Start founder clock #1 (e.g. natoshisakamoto — the launch-operator PC)
```
HONE_CHAIN_ID=hone \
HONE_ACCOUNT=natoshisakamoto \
HONE_POSTING_KEY=<natoshisakamoto posting PRIVATE seed hex, from its keystore> \
HONE_CLOCK=true \
HONE_MINER=false \
HONE_GENESIS_FILE=$PWD/rust/hone-node/genesis.json \
HONE_GENESIS_TIMESTAMP=1783191600000 \
HONE_DATA_DIR=/var/hone/clock-nato \
HONE_API_PORT=4242 HONE_P2P_PORT=6942 \
  ./target/release/hone-node
```
Note this node's **P2P multiaddr** from the startup log (`/ip4/<ip>/tcp/6942/p2p/<peerid>`).

### 3. Start founder clock #2 (e.g. shindevlin, or the Nebra) pointing at #1
```
HONE_CHAIN_ID=hone \
HONE_ACCOUNT=shindevlin \
HONE_POSTING_KEY=<shindevlin posting PRIVATE seed hex> \
HONE_CLOCK=true \
HONE_MINER=false \
HONE_GENESIS_FILE=$PWD/rust/hone-node/genesis.json \
HONE_GENESIS_TIMESTAMP=1783191600000 \
HONE_DATA_DIR=/var/hone/clock-shin \
HONE_BOOTSTRAP_PEERS=/ip4/<clock1-ip>/tcp/6942/p2p/<clock1-peerid> \
HONE_API_PORT=4242 HONE_P2P_PORT=6942 \
  ./target/release/hone-node
```

### 4. Confirm both are on the SAME chain and genesis
```
for n in <clock1>:4242 <clock2>:4242; do
  echo "$n:"; curl -s http://$n/api/node/info | grep -oE '"chain_id":"[^"]*"'
  curl -s http://$n/api/block/0 | grep -oE '"hash":"[^"]*"'
done
```
Both `chain_id` equal + both block-0 hashes equal the recorded genesis hash. If either
differs → they will fork. Stop and reconcile chain_id / genesis.json / timestamp.

---

## ★ VERIFICATION — the part the old runbook was missing ★

Run ALL of these. Do not declare launch on any single one — especially not the epoch number.

### V1 — clocks are actually registered (NOT zero)
```
curl -s http://localhost:4242/api/clock/registered
```
- **PASS:** `stakers ≥ 2`, `accounts` lists your founder clocks.
- **FAIL (the 2026-07-10 state):** `{"stakers":0,"accounts":[]}` → `HONE_CLOCK` isn't set,
  or the node isn't reaching the registration path. During bootstrap grace (first 100k
  epochs) a clock registers at stake 0 automatically; if it's still 0 registered after a few
  epochs, the clock role is not actually on.

### V2 — clock status shows in_quorum
```
curl -s http://localhost:4242/api/clock/status
```
- **PASS:** `in_quorum: true`, `store_registered: true`, `registered_count ≥ 2`,
  `registration` non-null, `observer: false`.
- **FAIL:** `in_quorum:false` / `registered_count:0` / `node_account:"genesis"`.

### V3 — ★ STATE IS ADVANCING (the decisive check) ★
```
curl -s http://localhost:4242/api/chain/state_root
```
- **PASS:** `state_root` is **non-zero** and its `epoch` is the **current** epoch (close to
  the wall-clock epoch).
- **FAIL (the trap):** `{"state_root":"0000…0000","epoch":0}` while `/api/node/info` shows a
  high epoch → **the chain is NOT live.** The epoch counter is lying to you; state is stuck
  at genesis. This is the single most important check in this document.

### V4 — validator snapshots are being recorded
```
E=$(curl -s http://localhost:4242/api/node/info | grep -oE '"epoch":[0-9]*' | grep -oE '[0-9]+')
curl -s "http://localhost:4242/api/chain/validators/$((E-2))"
```
- **PASS:** non-empty `validators`, `count ≥ 2`.
- **FAIL:** `"note":"no snapshot recorded for this epoch"` on recent epochs → not sealing.

### V5 — founder balances rise from ClockReward
```
for a in natoshisakamoto shindevlin; do curl -s http://localhost:4242/api/balance/$a; done
```
- **PASS:** balances **increase** epoch over epoch (during grace, ClockReward builds the
  clock's stake first, then spills to balance — so watch `clock/registered` stake climb too).
- **FAIL:** flat `0` across many epochs → no rewards emitted → not sealing.

### V6 — gossip is actually flowing (no topic mismatch)
Check the node log for the absence of:
```
Gossipsub publish on 'hone/entries' failed: NoPeersSubscribedToTopic
```
- **PASS:** that WARN is absent (peers share the chain_id gossip namespace).
- **FAIL:** it repeats every ~30s → chain_id mismatch between this node and its peers. Fix
  `HONE_CHAIN_ID` to the locked value and restart.

**Launch is real only when V1–V6 all pass.** V3 is the gate.

---

## Rollback / if it stalls
- **state_root stuck at 0 but epoch climbing:** you have the 2026-07-10 condition. Check V1
  (clocks registered?) then V6 (gossip topic match?). Almost always one of: `HONE_CLOCK`
  unset, chain_id drift, or only one clock up.
- **Nodes forked (different block-0 or state_root):** mismatched genesis.json / timestamp /
  chain_id. Stop both, reconcile to the single agreed genesis, wipe the FRESH data dirs,
  relaunch. A wrong key/param at genesis is permanent once the chain runs — treat a fork in
  the first epochs as "stop and fix," not "let it ride."
- **Sim traffic in a production log:** `HONE_WORK_GENERATOR` is set somewhere — unset it and
  restart that node.

---

## After V1–V6 pass (then, and only then)
1. Add the remaining role nodes (miners `HONE_WORKER=true`, storage `HONE_STORAGE=true`,
   sensors `HONE_SENSOR=true`, the Nebra/Flipper verticals) — now that seals land, their
   heartbeats earn real Layer-B rewards.
2. Run the public-surface checks in `docs/GO_LIVE_CHECKLIST.md` (that checklist tests
   *serving traffic* and only becomes meaningful once state is live; it is also stale — still
   says "HONE" — refresh it).
3. Address the **verifier-bond bug** (`reports/research/BUG-verifier-zero-bond-2026-07-09.md`)
   before the inference marketplace carries real value — on a state-live mainnet it is a live
   economic hole, not a testnet one.

---

## Coordinated-event note
Starting mainnet is a **coordinated founder event** — real vault wallets, a locked chain_id,
the `verify-vault` gate, and ≥2 founders bringing clocks up together. Per project rule it is
**never done solo**. This runbook is the mechanical script for that event; it is not an
invitation for one machine to self-start the chain.

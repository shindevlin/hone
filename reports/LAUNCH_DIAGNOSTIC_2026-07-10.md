# HONE Launch Diagnostic — 2026-07-10

**One-line status:** HONE is **time-live but not state-live**. Genesis time (2026-07-04
noon LA) has passed and a node is running, but **no clock is sealing, the chain is stuck at
block 0, and nothing has been emitted or earned.** Mainnet has not actually been started —
what is running is a dev/sim node. This is a startup/coordination gap, not a broken chain.

Evidence gathered live from the running node's HTTP API (`localhost:4242`) and its run log
on 2026-07-10. Every claim below is backed by an observed reading.

---

## The picture in one table

| Signal | Observed | Healthy launch would show | Verdict |
|---|---|---|---|
| Node process / API | responds, `version 1.2.2` | up | ✅ |
| Epoch counter | `17002` and climbing | climbing | ✅ (but see below) |
| **chain state_root** | `0x000…000 @ epoch 0` | non-zero, current epoch | ❌ **no state past genesis** |
| **Registered clocks** | `0` (`stakers:0, accounts:[]`) | ≥2 | ❌ **nobody sealing** |
| **Validator snapshots** | empty for every epoch queried | populated | ❌ |
| **is_clock** | `false` | `true` on sealers | ❌ **this node isn't a clock** |
| Founder balances (shindevlin/natoshisakamoto/josh) | `0 HONE` each | earning after seals | ❌ **zero emission** |
| total_emission | `null` | rising | ❌ |
| Accounts | 1164, but 1163 share ONE posting key, 0 funded | distinct funded participants | ⚠️ genesis name-reservations, not users |
| Gossip `hone/entries` publish | `NoPeersSubscribedToTopic` every 30 s | delivered to peers | ❌ **can't sync with peers** |
| Peers | 21–22 connected | connected AND subscribed | ⚠️ transport-connected only |
| Run-log freshness | last line 2026-07-09 02:47 (~37 h stale) | current | ❌ **event loop/logging silent** |

The epoch counter climbing is **not** proof of a live chain — epoch is computed from
wall-clock (`(now − genesis_ts)/30s`). It advances whether or not anyone seals. The
`state_root = 0 @ epoch 0` is the truth: **zero blocks with real state have been produced.**

---

## Root cause — a stack of startup gaps, not a chain failure

### 1. This is a dev/sim node, never launched as a production sealer
- **Startup line:** `hone-node starting — account=genesis chain=hone-testnet data="X:/hone-inference-data"`.
  It booted as the placeholder `genesis` account, on **testnet**, and **printed a brand-new
  throwaway mnemonic at boot** ("NEW HONE WALLET — WRITE DOWN YOUR MNEMONIC"). A real founder
  clock runs a founder wallet, not a boot-generated one.
- The **sim harness is running**: log shows `sim tick N (epoch 0): josh->bullship`,
  synthetic founder-to-founder transfers, and fake `phi-3`/`llama3-8b` inference jobs. That
  is the demo/dev loop, not real traffic. (It also errors: `sim: transfer failed: invalid
  nonce: got 0, expected 1`.)

### 2. `HONE_CLOCK` was never set → the node never attempts to seal
- Sealing is gated on `cfg.is_clock` (`main.rs:550,597`). API confirms `is_clock:false`,
  `store_registered:false`, `registration:null`. Grep of the entire run log finds **zero**
  clock/seal/register/quorum lines. The node is a passive observer by configuration.
- With **0 registered clocks**, the chain cannot advance state — the exact PoW-genesis
  deadlock the 100k-epoch bootstrap-grace period exists to break (a founder clock may
  register at stake 0 during grace and build stake from `ClockReward`). Nobody has done it.

### 3. chain_id drift → peers can't gossip chain entries
- Gossip topics are namespaced `{chain_id}/hone/entries` (`net.rs:266-278`). The node
  **launched as `hone-testnet` but the API now reports `hone`**, and discovery queried
  `honemesh.net/api/peers/bootstrap?chain_id=hone-testnet`. Whatever the 21 peers are
  subscribed to, this node's publish target does not match → `NoPeersSubscribedToTopic`
  every 30 s for days. Peers are connected at the transport/DHT layer but not on the same
  chain's gossip namespace. **Even a sealing clock here could not propagate seals.**

### 4. Log is ~37 h stale
- API answers (epoch is wall-clock derived) but the run log stopped at 2026-07-09 02:47.
  The node's event loop / logging may be wedged; a restart is likely needed regardless.

---

## What IS proven (real progress — don't lose this)
- The binary builds and runs (v1.2.2), holds 21+ peers, completes Kademlia DHT bootstrap.
- It tracks genesis time correctly and loads a real embedded model
  (`llama2-uncensored.gguf`, 743 GB disk).
- The sim exercises the full entry pipeline end-to-end (accounts, transfers, inference job
  post/verify). **The machine works.** The gap is *starting the real network*, not building it.

---

## Fix path — dependency-ordered (coordinated founder event; do NOT do solo)

> Per project rule, mainnet start is a coordinated founder event (real wallets, genesis
> chain_id, the verify-vault launch gate). This section is the plan, not an action.

1. **Decide and lock chain_id — `hone` (mainnet) vs `hone-testnet`.** The drift is the
   direct cause of the gossip failure. This is genesis-adjacent; get it right once, and
   launch every node with the same value. Confirm the genesis block-0 hash matches the
   intended chain (`98e3c1b0…` for the re-genesis).
2. **Launch ≥2 founder clocks**, each:
   - a **founder wallet** (from the vault — not the boot-generated throwaway),
   - `HONE_CLOCK=true`,
   - the **same** chosen chain_id,
   - persistent uptime.
   During bootstrap grace they register at stake 0 and accrue stake from `ClockReward`.
   Quorum minimum is 2 unique sealers (`HONE_CLOCK_QUORUM`), so **two** clocks on the same
   chain_id is the floor to get state past epoch 0.
3. **Turn OFF the sim** on production nodes (no `sim tick` in a mainnet log).
4. **Verify state advances:** `state_root` becomes non-zero and current; `clock/registered`
   shows the founder clocks; founder balances begin rising from `ClockReward`.
5. **Then** the GO_LIVE public-surface checklist (`docs/GO_LIVE_CHECKLIST.md`) becomes
   meaningful — it tests serving traffic, which only matters once the chain is state-live.
   (That checklist is stale — still says "HONE" and covers testnet serving, not chain
   start; it should be refreshed after step 4.)

---

## The honest headline

The burners are lit and the recipe is written, but **this pot is a dev demo simmering on
epoch 0 — not the mainnet dinner service.** "Genesis timestamp arrived" is baked into the
binary and fires on July 4 whether or not a human starts the network. **No human has started
the network.** The single highest-leverage action is step 2: get two founder clocks sealing
on one agreed chain_id.

_Diagnostic by inspection of the live node + run log, 2026-07-10. Read-only; nothing changed._

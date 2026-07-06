---
title: HONE Genesis — Launch-Day Runbook (July 4 2026, noon LA)
description: The mechanical go/no-go checklist for launching the recoverable chain
author: Shin Devlin
---

# Genesis v2 Launch Runbook

**Launch: 2026-07-04 12:00 noon Los Angeles = 19:00 UTC. Timestamp `1783191600000`, chain `hone`.**

This is the ordered checklist for the launch operator (the natoshisakamoto PC).
Do these in order. **The chain has launched and failed four times because keys
were unrecoverable — the whole point of this launch is that it does not happen a
fifth time. `verify-vault` (step 4) is the hard gate. Do not skip it.**

---

## The one rule
> **No account is signable unless its keystore exists and its key matches genesis.**
> `verify-vault` proves this. If it does not pass, DO NOT LAUNCH.

---

## T-minus (before 19:00 UTC) — build & stage

### 1. Create every operated wallet (recoverable from the start)
For each account, with a strong password you SAVE:
```
HONE_WALLET_PASSWORD='<strong password — SAVE IT>' \
  hone wallet new --account <name> --vault wallets
```
Accounts: `shindevlin` (founder / root owner — the MOST important to recover, it
owns freeport/verasens/linkgit), `natoshisakamoto`, `bullship`, `freeport`,
`linkgit`, `verasens`, `hone-market`, `hone-relay`.

**For EACH one: write down the recovery phrase it prints (shown once).** This is
the layer-2 backup. If you skip it and later lose the password, that account is
gone — the exact failure we are ending.

### 2. Build the index and eyeball it
```
hone wallet index --vault wallets
cat wallets/INDEX.md
```
Every account must show **✓ yes** under "Recoverable keystore". Any ✗ = stop.

### 3. Write genesis.json with the vault's posting pubkeys
Use each account's posting pubkey (from `wallets/INDEX.md` or
`hone wallet pubkeys --keystore wallets/<name>.keystore.json`):
```jsonc
{ "genesis_timestamp": 1783191600000, "chain_id": "hone",
  "accounts": {
    "<name>": { "keys": { "posting": "<pubkey>" }, "balance": <dreams> },
    "__treasury__": { "balance": 0 }, "__recycle_fund__": { "balance": 0 },
    "__testnet_fund__": { "balance": 0 } } }
```
Update the timestamp constant in ALL THREE (CI gate check-constants.yml enforces):
`rust/hone-node/src/config.rs`, `rust/hone-node/genesis.json`, `docs/CHAIN_CONSTANTS.md`.

### 4. ★ THE GATE — verify-vault ★
```
hone wallet verify-vault --vault wallets --genesis rust/hone-node/genesis.json \
    --require-accounts rust/hone-node/genesis-required-accounts.txt
```
- **Exit 0 / "Safe to launch"** → proceed.
- **Exit 2 / FAIL** → one of three things is wrong, all launch-blocking:
    - a genesis account has NO recoverable keystore,
    - a genesis account's key does NOT match its vault key,
    - a REQUIRED account is MISSING from genesis entirely (`--require-accounts`
      enforces the canonical list — this catches an account left off by mistake,
      the exact gap that was previously caught only by human eye).
  Fix and re-run. Launching now would relaunch the exact bug. Do not proceed.

### 5. ★ BACK UP THE VAULT OFF THIS MACHINE ★
Recoverability is worthless if it lives on one disk that can die.
- Copy `wallets/` to a second physical location (USB, second machine) — it is
  already encrypted (keystores), but keep it private.
- Confirm every recovery phrase from step 1 is written down on paper, offline.
- (Optional) once the chain is up, `hone wallet backup --keystore <f> --node <url>`
  for relay backup — but the off-machine copy above is the must-have for launch.

### 6. Build + stage the node binaries (do NOT start yet if before 19:00)
- x86_64 primary node: `cargo +1.90.0 build --release -p hone-node`
- Nebra aarch64:
  ```
  rustup target add aarch64-unknown-linux-gnu
  CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
    cargo build --release -p hone-node --target aarch64-unknown-linux-gnu
  ```
  scp the binary + genesis.json to the Nebra.

### 7. Fresh-genesis smoke test (throwaway dir) and RECORD the block-0 hash
```
HONE_DATA_DIR=/tmp/gv2-smoke HONE_CHAIN_ID=hone \
  HONE_GENESIS_FILE=$PWD/rust/hone-node/genesis.json \
  HONE_GENESIS_TIMESTAMP=1783191600000 HONE_API_PORT=4299 HONE_P2P_PORT=6999 \
  ./target/release/hone-node &
sleep 8
curl -s http://localhost:4299/api/block/0 | grep -o '"hash":"[^"]*"'
```
**Record this hash.** Every genesis node MUST reproduce it. (Pre-genesis the node
reports `"epoch":0` — that is correct, not a failure.) Kill the smoke node.

---

## T-zero (19:00 UTC / noon LA) — launch

### 8. Start the primary node on a FRESH data dir
Fresh `HONE_DATA_DIR` (the node refuses to re-init over an existing block 0).
Set the v2 env (chain_id hone, ts 1783191600000, the account's posting PRIVATE
key from its keystore). Starting a few minutes early is safe — the node waits at
epoch 0 until 19:00 UTC, then begins sealing.

### 9. Start the Nebra node (same genesis, same ts, same chain_id)
`HONE_MINER=false` (ARM hotspot: clock + P2P). Point its bootstrap at the primary.

### 10. ★ Confirm both nodes agree on genesis ★
```
curl -s http://<primary>:4242/api/block/0 | grep -o '"hash":"[^"]*"'
curl -s http://<nebra>:4242/api/block/0  | grep -o '"hash":"[^"]*"'
```
Both MUST equal the hash recorded in step 7. If they differ → mismatched
genesis.json / timestamp / chain_id → they will fork. Stop and reconcile.

### 11. Watch the chain cross epoch 0 at 19:00 UTC
```
watch -n5 'curl -s http://localhost:4242/api/node/info | grep -o "\"epoch\":[0-9]*"'
```
Stays 0 until 19:00 UTC, then climbs ~1 per 30s. That is the chain alive on the
anniversary.

---

## Post-launch (not blocking)
- Wire the node-side `POST/GET /api/keystore/backup` route (Layer 3 server half —
  client already ships as `hone wallet backup/restore`).
- Register real API keys (e.g. bullship inference) now that we hold signable keys.

## If anything fails the gate
A wrong or missing key at genesis is **permanent** once the chain runs. It is
always cheaper to delay the launch an hour and fix the vault than to relaunch a
fifth time. `verify-vault` green is the only signal that matters.

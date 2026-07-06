---
title: BTCPC → [NEW_NAME] Rebrand — re-genesis plan
description: Rebrand the chain to a new name via a fresh genesis (same July-4 anchor, same keys), and update the whitepaper + all docs
author: Shin Devlin
status: DRAFT — pending name decision (HONE under IP/crypto research)
---

# Rebrand Plan — new chain name via re-genesis

> **Decision (Shin):** rebrand the chain to a NEW name, backdated to the same
> July-4-2026 noon-LA anchor, keeping the SAME account keys from the vault.
> Update the whitepaper + all documentation to the new name. Re-genesis (not a
> live chain_id migration) — cleaner for a chain this young (~1 day old).

## LOCKED NAMING (2026-07-05, after IP/crypto/domain research)

| Element | New value | Was |
|---|---|---|
| **Project / brand** | **HONE** | BTCPC |
| **Token / ticker** | **HONE** | BTCPC |
| **chain_id** | **`hone`** (no version number) | hone |
| **Base unit (smallest)** | **hunit** (1 HONE = 10,000,000,000 hunits) | dream |
| **Domain** | **honemesh.net** (register at registrar) | — |
| **Genesis timestamp** | 1783191600000 (July 4 2026 noon LA) — unchanged | same |
| **Account keys** | same 11 keys from the vault — unchanged | same |
| **Balances** | ZEROED (fresh genesis) — Shin approved | reset |

**Constant renames:**
- `MAINNET_CHAIN_ID = "hone"` → `"hone"`
- `NATIVE_TOKEN = "BTCPC"` → `"HONE"`
- `HUNITS_PER_HONE` → `HUNITS_PER_HONE` (value unchanged: 10^10)
- the unit string `"dreams"` → `"hunits"`; `"dream"` → `"hunit"`
- brand literal `BTCPC` → `HONE` (docs/user-facing); keep in historical/changelog context

**Research verdict (why HONE + HONE):** HONE ticker is clean on every check
(no existing HONE token; you're the FIRST HONE chain). HONE brand: honemesh.net
+ .io available (.com taken). Note: the "mesh" crypto namespace is crowded
(Mesh Connect $1B, Polymesh, Meshswap) — Shin chose HONE knowing this. Prior
rejects: HoneLabs (crypto-dev namesake), DataMesh (generic term + Deutsche-Bank
paytech), Honedata (Hondata sensor-app collision).

## Why re-genesis (not live migration)
The chain is ~1 day old (epoch ~2270). Its only durable state is the 11 founder
accounts, whose keys are all in the vault and whose pubkeys are known. A live
`HONE_CHAIN_ID_MIGRATION` would work but must be perfectly synchronized across
every node (chain_id is in every signature) or the network forks. A fresh genesis
under the new name, seeded with the SAME account keys, is simpler and gives a
clean brand break — no residual "hone" anywhere. Balances reset to genesis
allocation (they were near-zero-earned anyway; founder rewards restart).

## What stays the same
- **Genesis timestamp:** `1783191600000` (July 4 2026 noon LA) — the anniversary.
- **Account keys:** the EXACT keys already in the vault (HONE_SECRETS_BACKUP).
  No new keystores, no key handling — the pubkeys are already known and in the
  current genesis.json. shindevlin `c97a9f20…`, natoshisakamoto `29e22bbd…`,
  josh `bcc9d57d…`, bullship `bc52f8fd…`, freeport, linkgit, verasens,
  btcpc-market, btcpc-relay, btcpcbot, btcpcwalletbot.
- The 11 accounts + system funds (__treasury__, __recycle_fund__, __testnet_fund__).

## What changes
- **chain_id:** `hone` → `[new-chain-id]` (in `crates/hone-types/src/lib.rs`
  MAINNET_CHAIN_ID, `genesis.json`, `config.rs` default, `CHAIN_CONSTANTS.md`).
- **Genesis block payload:** the launch proclamation text (currently "BTCPC v2 …")
  → "[NEW_NAME] …". This changes block-0 hash → a NEW canonical hash (expected;
  it's a new chain). Record it; all nodes must reproduce it.
- **Project name** everywhere: whitepaper, README, all docs, the node's user-
  facing strings, the ticker/token name if changing.
- New data dir (fresh genesis needs an empty HONE_DATA_DIR).

## Execution order
1. **Confirm the name** (HONE pending research) + the ticker.
2. **Rename constants:** chain_id + MAINNET_CHAIN_ID + genesis_timestamp already
   correct; update the proclamation string in `genesis.rs`; update
   `CHAIN_CONSTANTS.md`. CI constant-drift gate must pass.
3. **Rebuild genesis.json** with the same 11 account pubkeys + new chain_id.
   (Reuse the existing pubkeys — copy from current genesis.json, change only
   chain_id.) Since keys are unchanged, no vault access needed.
4. **Rebuild the node binary** (candle etc. already in).
5. **Fresh-genesis smoke test** in a throwaway data dir → record the NEW block-0
   hash. Every node must reproduce it.
6. **Whitepaper + docs rebrand:** global BTCPC → [NEW_NAME] where it's the brand
   (keep "BTCPC" only in historical/changelog context). Update genesis section,
   ticker, one-pager, README, website.
7. **Coordinated cutover:** all founder nodes (beastly/shindevlin, natoshisakamoto,
   Nebra/josh) stop, wipe data dir, restart on the new genesis. Confirm all
   produce the same new block-0 hash.
8. **Clients/consumers:** update chain_id in wallets, bullship .env, the
   integration manifest (chain_id field), any bots.
9. **Re-register live state:** bullship's inference API key + funding (the sign-
   requests go to Shin/founder per the no-autonomous-transfer rule). Any live
   sensor (josh/test) re-registers on the new chain.

## Gotchas
- **Block-0 hash WILL change** (new proclamation + chain_id in payload) — that's
  correct for a new chain, but every node must rebuild + wipe + restart together,
  exactly like the July-4 launch. Reuse GENESIS_LAUNCH_RUNBOOK.md's verify steps.
- **Balances reset.** If any account earned meaningful BTCPC on hone that must
  carry over, note it — a fresh genesis does NOT preserve balances (only keys +
  identities). shindevlin had ~1.3B dreams earned; decide if that matters
  (likely not — rewards restart).
- **The integration manifest** hardcodes chain_id in places — regenerate it.
- **Ticker/token rename** (if BTCPC the ticker changes too) touches emission
  docs + any exchange/bridge references.

## Open questions
1. The NAME (HONE viability under research) + the TICKER symbol.
2. Does the PROJECT rename fully (BTCPC → HONE everywhere) or does the chain get a
   new name while some legacy "btcpc" identifiers stay? (Recommend full rename for
   a clean brand.)
3. Preserve any hone balances, or clean genesis reset (recommend clean)?

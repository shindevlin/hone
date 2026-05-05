# Chain Sync Notes

Branch: `chain-sync-hardening`

## What was blocking sync

1. `src/p2p/network.js`
   - The retry bind path for `EADDRINUSE` created a fresh `WebSocket.Server` but did not consistently reattach the connection handler to the retry instance.
   - Result: the process could be listening on the fallback port while accepting no peers.

2. `src/p2p/protocol.js`
   - The `RESPONSE_BLOCKS` path wrote block files, but it did not replay the block payload into `stateStore`, `stateManager`, or `nodeRegistry`.
   - Result: a node could appear synced on disk while its live balance/state/proof caches were still stale.

## Why that mattered

- Miners rely on live state for reward proposals, work attestations, and finalization decisions.
- If blocks arrive but payload state is not applied, the node can:
  - advertise the wrong height or appear stale,
  - miss expected proofs and rewards,
  - fail to participate correctly in finalization,
  - look "synced" in files while the runtime state is still behind.

## Fixes applied

- Made the P2P server retry path attach handlers to every retry instance.
- Added a shared block-payload replay helper for received blocks.
- Applied ledger entries, proofs, and finalized epoch metadata when block sync succeeds.
- Added regression coverage for both the retry path and the block replay path.

## Verification

- `npx jest tests/p2pNetworkServerRetry.test.js --runInBand`
- `npx jest tests/p2pSyncReplay.test.js --runInBand`
- `npx jest --runInBand`

---

## Known consensus bypass (Rust node — flag for pre-mainnet fix)

**Filed: 2026-05-05**

Two categories of Rust node writes currently bypass gossip and write local RocksDB only:

### 1. Faucet transfers (`api.rs` — `post_faucet_claim`)

`store.debit(FAUCET_ACCOUNT)` + `store.credit(recipient)` are direct balance mutations
via the `Store` API. They are not submitted as `LedgerEntry::Transfer` entries, not
gossiped to peers, and not applied through `Chain::validate_and_apply`. On a single-node
testnet this is invisible. On a multi-node testnet, the faucet node's balance state
diverges from peers — they see a recipient balance increase with no corresponding on-chain
entry.

**Fix required before mainnet**: post faucet transfers as signed `LedgerEntry::Transfer`
entries through the normal gossip path (same as any other transfer), sourced from the
`__testnet_fund__` account which holds a pre-mined reserve.

### 2. Miner capability records (`api.rs` — `post_node_capability`)

`state_set("miner_caps:{account}", ...)` writes to `CF_META` locally. This is intentional:
capability records are self-reported node metadata, not consensus-sensitive state. No
on-chain proof or cross-node sync is needed — capability queries hit whichever node the
caller reaches, and nodes only serve their own miners' records.

**No fix needed.** Document in API spec that `/api/node/capability` is local-only.


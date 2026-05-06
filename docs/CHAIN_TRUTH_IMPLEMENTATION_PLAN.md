# BTCPC Chain Truth & P2P Hardening

This phase turns the agreed BTCPC chain model into code:

- chain truth lives on-chain only
- replay is a cache/bootstrap step, not the source of truth
- clock nodes learn height from on-chain P2P agreement
- if exactly two live nodes are present, they determine height together deterministically
- localhost is never valid for chain truth
- storage data stays segregated and encrypted on storage nodes

## Goals

1. Make every chain-facing read path agree on the same chain tip.
2. Remove localhost from peer identity, seed configuration, and node publication.
3. Keep live node identity visible without inventing truth locally.
4. Make health checks report truth-bearing status only when the network actually qualifies.
5. Preserve the current replay model as a bootstrap/cache layer.

## Non-goals

- No new consensus algorithm.
- No local-only fallback for chain truth.
- No user-visible raw storage browsing.
- No new database for chain state.

## Phase Steps

### Step 1. Canonicalize truth reads

- Audit all endpoints that report epoch, height, or finality.
- Ensure they read chain height from the block/state source of truth.
- Keep replayed state as a cache that mirrors the chain, not as a competing view.

### Step 2. Remove localhost from truth paths

- Remove loopback peers from seed config and persisted peer discovery.
- Reject localhost and local-only addresses when registering or publishing nodes.
- Keep localhost only for private developer tooling that does not affect chain truth.

### Step 3. Make clock consensus deterministic

- Require at least two live connected nodes before claiming truth.
- If exactly two live nodes are connected, height must converge deterministically.
- Do not let a single process self-authorize chain progress.

### Step 4. Publish node identity correctly

- Persist P2P addresses when nodes register.
- Surface real connectable addresses in the node list.
- If the live process knows a better address than the replayed chain snapshot, show it as a live view only, not as a new chain fact.

### Step 5. Keep storage segregated

- Continue to keep storage node payloads encrypted and isolated.
- Ensure the public API exposes only allowed storage status, not raw internals.

### Step 6. Tighten health checks

- Health should require:
  - API up
  - clock running
  - miner running
  - peers connected
  - chain tip aligned
  - no localhost truth path
- Health must fail closed when chain truth is ambiguous.

### Step 7. Verify with tests

- Add or update tests for:
  - epoch/current follows chain height
  - node list publishes a real P2P address
  - localhost peers are rejected
  - two-node truth works
  - health fails when one-node truth is attempted

## Files Likely To Change

- `src/controllers/nodeController.js`
- `src/controllers/publicRoutes.js`
- `src/mining/miner.js`
- `bin/btcpc-clock`
- `src/p2p/address.js`
- `src/p2p/protocol.js`
- `src/p2p/network.js`
- `src/chain/clockConsensus.js`
- `src/chain/stateStore.js`
- `src/services/ledger.js`
- `.env`
- `tests/nodeControllerHealth.test.js`
- `tests/p2pAddress.test.js`
- `tests/p2pDiscovery.test.js`
- `tests/clockConsensus.test.js`

## Verification

1. Run the targeted tests for the changed surface.
2. Restart the live API, clock, and miner services.
3. Confirm:
   - `/api/node/epoch/current` matches chain height
   - `/public/machine-status` matches the same height
   - no live log shows `localhost` for chain truth
   - live node list reflects real endpoints when available

## Current State

Already started:

- API epoch reads chain height instead of wall clock.
- Miner and clock startup preserve advertised P2P addresses.
- Loopback seed peers were removed from `.env`.
- A regression test exists for the chain-height epoch path and node address fallback.


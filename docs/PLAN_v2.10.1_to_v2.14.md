I have enough now. Producing the plan.

---

# HONE v2.10.1 → v2.14.x — Decentralized Commerce + Compute Hosting Plan

Author: planning agent
Target: Shin Devlin / HONE v2.10.0 baseline (commit b47d74a)
Status: draft for review — no code changes made

---

## 1. Architecture overview

These five phases turn HONE from "blockchain + AI inference mining" into "blockchain + AI inference + content-addressed object store + stateless then stateful compute hosting" — all reusing the same primitives: ledger entries flow through `stateStore`, escrow holds value, reputation scores behavior, slashing punishes cheating, mining proofs already model challenge/response.

The five phases stack like this:

```
 v2.14  STATE_SNAPSHOT / STATE_TRANSFER         (stateful compute)
           depends on v2.13 + v2.11
 v2.13  SERVICE_DEPLOY / SESSION_START/END      (stateless compute)
           depends on v2.11 + v2.10.1
 v2.12  BLOCK_SIZE_CAP + fee market             (scaling discipline)
           depends on nothing; can parallel with v2.11
 v2.11  HONE-FS: BLOB_STORE_COMMIT/SERVE/CHAL   (object store)
           depends on v2.10.1
 v2.10.1 Commerce HTTP routes                   (makes v2.10 reachable)
           depends on commit b47d74a only
```

How the core primitives are reused across phases:

- **stateStore** — the single source of read truth. Each phase adds Maps (`blobs`, `services`, `sessions`, `snapshots`) and dispatcher cases to `applyEntry`. No new database. Finality snapshots grow to carry the new Maps in `extended_state`.
- **ledger.recordX + pendingEntries** — every new action is a `_entry() + _persist()` pair. `applyEntry` mutates stateStore synchronously, the miner flushes `pendingEntries` into the next block payload. The same machinery that persists STAKE, TRANSFER, ORDER_PLACE persists BLOB_STORE_COMMIT, SERVICE_HEARTBEAT, STATE_SNAPSHOT.
- **escrow.js** — reused as a generic "lock funds, release to party X on success, refund on failure" primitive. Phase 2.10.1 ties it to ORDER_PLACE; 2.11 ties it to BLOB_STORE_COMMIT (seller → storage host); 2.13 ties it to SESSION_START (user → deployer→host split); 2.14 inherits the same flow. No changes to `lockFunds` signature — the escrow_id just becomes a foreign key on orders/blobs/sessions.
- **HONE-FS (v2.11)** is the glue that appears in every later phase. Large off-chain artifacts — product images, order fulfillment evidence, service binaries, state snapshots — all live as `cid → bytes` at `~/.hone/blobs/<cid_prefix>/<cid>` on storage hosts. On-chain we only ever reference CIDs. Products already carry `content_cid`, orders already accept `fulfillment_cid`; v2.11 gives those fields teeth by making content actually retrievable from peers.
- **Reputation (v2.10)** is extended by every later phase. Compute hosts (v2.13) get auto-votes from verifier uptime probes. Storage hosts (v2.11) get auto-votes from challenge success rate. Stateful hosts (v2.14) get auto-votes from successful snapshot verification. Everything flows through `recordReputationVote` with `target_type: "miner"` or new `target_type: "host"` — same Map, same math.
- **Mining challenge/response** — already implemented for inference in `inference/handler.js` and `inference/verifier.js` (VERIFY_REQUEST / VERIFY_RESPONSE). v2.11 reuses this pattern for BLOB_CHALLENGE. v2.13 reuses it for heartbeat probes. v2.14 reuses it for state-key spot checks. One verifier panel, multiple challenge types.
- **Slashing (v2.10 slashing.js)** — already has `recordOffense(account, offenseType, evidence)`. Each new phase adds offense types: `blob_serve_failed`, `service_downtime`, `snapshot_missing`, `state_mismatch`. No new slashing code — just new strings + evidence shapes.
- **Block files as source of truth** — every phase stays inside the `ledger.record → stateStore.apply → pendingEntries → blockStore.writeBlock` pipeline. Nothing is written outside of blocks. On-disk blob data at `~/.hone/blobs/` is not chain state — it's just the addressable payload; the commitment (the CID) is in the block. If a host loses their blob dir they lose their bond, not the chain's view of it.
- **Finality snapshots** — v2.10 left a latent bug: `stateManager.generateFinalitySnapshot` builds `extended_state` with only `tokens`, `projects`, `extra_balances`, even though `stateStore.hydrateFromFinality` already reads `stores/products/orders/reputation/reputation_votes/delegations`. This gets fixed in v2.10.1 as a prerequisite and the pattern is extended phase-by-phase.

Hierarchical picture of what each layer knows:

```
┌──────────────────────────────────────────────────────────────────┐
│ HTTP routes (v2.10.1)   P2P handlers (v2.11–v2.14)               │
│   JWT / API key auth      BLOB_CHALLENGE, SERVICE_HEARTBEAT, …   │
├──────────────────────────────────────────────────────────────────┤
│ ledger.recordX                                                   │
│   _entry() → stateStore.applyEntry → pendingEntries.push         │
├──────────────────────────────────────────────────────────────────┤
│ stateStore (in-memory cache)                                     │
│   balances, accounts, stakes, escrows, stores, products, orders, │
│   reputation, blobs (v2.11), services (v2.13), sessions (v2.13), │
│   snapshots (v2.14)                                              │
├──────────────────────────────────────────────────────────────────┤
│ miner.scheduleFinalization → flushPendingEntries → writeBlock    │
│   + enforces 1 MB cap (v2.12), fee-sorted selection (v2.12)      │
├──────────────────────────────────────────────────────────────────┤
│ blockStore → data/blocks/block-NNNNNNNN.bin                      │
│   + finality-NNNNNNNN.bin with extended_state (v2.10.1+)         │
├──────────────────────────────────────────────────────────────────┤
│ Off-chain payload: ~/.hone/blobs/<cid>  (v2.11) — addressable   │
│ Off-chain runtime: serviceHost subprocess wrappers (v2.13+)      │
└──────────────────────────────────────────────────────────────────┘
```

The important invariant: **the on-chain state never depends on the off-chain payload being present.** A blob commitment is just a promise by a host, and the chain slashes the host if the promise isn't kept. A service heartbeat is just the host saying "I'm still here" — the chain doesn't care about the in-process state, only about the ledger record.

---

## 2. Per-phase breakdown

### Phase v2.10.1 — Commerce HTTP routes

**Goal**: expose every v2.10 commerce ledger entry as an authenticated HTTP endpoint so wallets, bots and the Telegram frontend can drive store/product/order/reputation flows without writing ledger code directly.

**New files**:
- `src/routes/commerceRoutes.js` — stores, products, orders, reputation endpoints
- `src/middlewares/validateCommerce.js` — shared schema helpers (product_id regex, price bounds, category whitelist)
- `tests/commerceRoutes.test.js` — supertest-based route tests

**Modified files**:
- `src/index.js` — mount `app.use("/api/commerce", commerceRoutes)` next to `projectRoutes`
- `src/chain/stateManager.js` — extend `generateFinalitySnapshot().extended_state` to include `stores`, `products`, `orders`, `reputation`, `reputation_votes`, `delegations` (closes the v2.10 finality gap; `hydrateFromFinality` already reads them)
- `src/services/ledger.js` — add convenience wrappers `openStoreWithStake(seller, storeData, capacity, stableToken, stablePaidUsd, epoch)` and `placeOrderWithEscrow(buyer, seller, productId, quantity, unitPrice, token, epoch)` that atomically sequence the paired STAKE + TRANSFER + STORE_OPEN, or ESCROW_LOCK + ORDER_PLACE, so routes don't reimplement that order

**New ledger entry types**: none — all v2.10.0 entries stay. Only new wrapper functions.

**New stateStore Maps + dispatcher cases**: none.

**New recordX functions in ledger.js** (signatures, all `async`):
- `openStoreWithStake(seller, storeData, capacity, stableToken, stablePaidUsdCents, epoch)` → returns `{ storeEntry, stakeEntry, paymentEntry }`
- `placeOrderWithEscrow(buyer, seller, productId, quantity, unitPrice, token, epoch)` → returns `{ orderId, escrowId, orderEntry, escrowEntry }`; generates `orderId` via `crypto.randomBytes(12).toString('hex')` and `escrowId = 'order:' + orderId`

**HTTP routes added** (all under `/api/commerce`):
- `POST /stores` (JWT) — open a store. Body: `{ name, banner_cid, description_cid, categories, capacity, stableToken }`. Calls `openStoreWithStake`. Returns 201.
- `PATCH /stores/me` (JWT) — update your store metadata.
- `POST /stores/me/capacity` (JWT) — buy additional capacity. Body: `{ additionalCapacity, stableToken }`.
- `DELETE /stores/me` (JWT) — close store.
- `GET /stores` — browse. Query: `status`, `category`, `limit`, `offset`, `sort=rep|newest|sales`.
- `GET /stores/:seller` — single store view.
- `POST /products` (JWT) — create product. Body includes `product_id` (client-generated UUID), `title`, `description_snippet`, `content_cid`, `category`, `price`, `token`, `stock`. Returns 201 on success, 409 on capacity exceeded, 403 on missing store.
- `PATCH /products/:productId` (JWT) — update own product.
- `DELETE /products/:productId` (JWT) — delist.
- `GET /products` — browse with filters `{ seller, category, status, min_price, max_price }`.
- `GET /products/:productId` — single product.
- `POST /orders` (JWT) — place order. Body: `{ seller, productId, quantity }`. Unit price/token resolved server-side from stateStore to prevent tampering. Calls `placeOrderWithEscrow`. Returns 201 with `{ orderId, escrowId, total }`.
- `POST /orders/:orderId/fulfill` (JWT, seller) — mark fulfilled. Body: `{ fulfillment_cid }`. On success also triggers `ledger.recordEscrowRelease` to seller (or waits until delivery confirmation — see invariant below).
- `POST /orders/:orderId/delivered` (JWT, buyer) — confirm. Also triggers escrow release to seller if not already released, and auto-casts a neutral (+1) reputation vote unless dispute.
- `POST /orders/:orderId/cancel` (JWT, buyer or seller) — cancel, refunds escrow.
- `POST /orders/:orderId/dispute` (JWT, buyer) — dispute; escrow stays locked pending resolution.
- `GET /orders/me` (JWT) — list my orders (buyer or seller view via `?role=buyer|seller`).
- `GET /orders/:orderId` (JWT) — single order, only accessible to buyer or seller.
- `POST /reputation/votes` (JWT) — cast a vote. Body: `{ target_type, target_id, vote, weight, memo }`. Server clamps weight based on caller's stake / completed order count.
- `GET /reputation/:target_type/:target_id` — read aggregate.
- `GET /commerce/pricing?currentCapacity=N&additionalCapacity=M` — bonding curve quote (read-only, no auth).

All routes follow `projectRoutes.js` conventions: `sanitizeString`, `sanitizeAmount`, `rejectObjectInputs`, `validAccountName`, 400 on validation failure, 401 on missing auth, 403 on wrong ownership, 404 on missing entity, 409 on chain invariant failure, 500 only on unexpected throws.

**Chain invariants to enforce at the route layer (in addition to what `applyEntry` already silently enforces)**:
- Store open: caller must have balance ≥ required HONE stake, must not already have an active store.
- Product create: caller must own an active store; current active-product count < capacity; `product_id` is a fresh UUID (not already in stateStore).
- Order place: product must be active and in stock; unit price/token must match current stateStore values (anti-race); buyer ≠ seller; buyer has balance ≥ total.
- Order fulfill: only the seller; order must be in `placed`.
- Order delivered: only the buyer; order must be in `fulfilled`.
- Order cancel: only before fulfillment; refunds escrow atomically.
- Order dispute: only the buyer; only after placed; escrow must still be locked.
- Reputation vote: voter must have completed at least one order with that target (for store/product votes) OR have non-zero stake (for miner votes); weight ≤ min(stake, completed_orders × 10, 100).
- Escrow release ordering: seller is paid only after buyer delivers OR after a configurable auto-release delay (default 7 days, enforced by a periodic sweep — reuses `sweepEscrows` pattern with a new `autoReleaseOrders` scan).

**Reuse of existing code**:
- `middlewares/auth.js` `authenticateToken` for JWT.
- `middlewares/apiKeyAuth.js` `authenticateApiKey` for integration-style callers that want to operate a store programmatically (e.g. a fulfillment bot).
- `middlewares/validate.js` `rejectObjectInputs`, `sanitizeString`, `sanitizeAmount`, `validAccountName`.
- `services/escrow.js` `lockFunds` (unchanged signature, just called with `requestId = 'order:' + orderId`).
- `services/ledger.js` all existing `recordX` commerce functions.
- `chain/stateStore.js` all existing getters.

**Tests to write** (`tests/commerceRoutes.test.js`):
- auth required on every mutating route (401 when no token)
- open store happy path (201 + state visible)
- open store with insufficient balance (400)
- open store twice (409)
- create product with no store (403)
- create product beyond capacity (409)
- browse products with filters
- full order flow: place → fulfill → delivered, assert escrow released
- cancel order refunds escrow
- dispute locks escrow, then appeal route resolution (existing)
- reputation vote respects weight clamp
- reputation vote from user with no completed orders is rejected
- auto-release after N epochs via `autoReleaseOrders`
- 1MB request body rejected (helmet already does this)

**Verification steps**:
- `npm test` — all commerce.test.js + commerceRoutes.test.js green
- `npm run smoke:api` — append a commerce flow smoke section (open store, list product, place order, fulfill, deliver)
- Manual: start node, `curl -X POST /api/commerce/stores ...`, verify `stateStore.getStore('alice')` via explorer endpoint

**Rough effort**: medium.

---

### Phase v2.11.x — HONE-FS content-addressed blob store

**Goal**: give HONE a native, paid, slashable object store where sellers/deployers pay hosts to store and serve CID-addressed blobs for a committed number of epochs.

**Planned sub-versions**:
- `v2.11.0` — ledger entries, stateStore maps, CID helper, disk store, HTTP serve endpoint (no challenge yet — trust-based)
- `v2.11.1` — P2P BLOB_CHALLENGE / BLOB_PROOF_OF_RETRIEVABILITY handler, verifier integration
- `v2.11.2` — slashing hookup + auto-reputation from challenge success rate
- `v2.11.3` — blob expiry + reclaim (host escrow released at end of term)

**New files**:
- `src/services/honefs/cid.js` — compute CIDv1 (multihash sha256 + base32) over arbitrary Buffer, deterministic.
- `src/services/honefs/blobStore.js` — disk layer: `put(cid, buffer)`, `get(cid) → buffer|null`, `has(cid)`, `delete(cid)`, `listLocal()`. Path convention: `~/.hone/blobs/<cid[0..2]>/<cid>`.
- `src/services/honefs/hostRunner.js` — starts a background loop that serves challenges over P2P, uploads evidence (BLOB_SERVE_PROOF) periodically, and refuses new commits when disk is full.
- `src/services/honefs/challenge.js` — verifier-side random chunk request, deterministic seed per (epoch, cid).
- `src/routes/blobRoutes.js` — HTTP endpoints: `POST /api/blobs` (upload), `GET /api/blobs/:cid` (public serve), `POST /api/blobs/:cid/commit` (deployer pays for storage), `GET /api/blobs/:cid/status` (see hosts + remaining epochs).
- `tests/honefs.test.js` — unit tests for CID determinism, blobStore roundtrip, challenge/response, ledger flow, slashing trigger.

**Modified files**:
- `src/chain/stateStore.js` — add `blobs` Map + dispatcher cases (see below); add getters; add to `snapshot()`, `resetAll()`, `hydrateFromFinality().extended_state.blobs`; extend `_entryKey` with `blob_data.cid`.
- `src/services/ledger.js` — new recordX functions (see below).
- `src/chain/stateManager.js` — include `blobs` in `generateFinalitySnapshot().extended_state`.
- `src/p2p/protocol.js` — add `MESSAGE_TYPES.BLOB_CHALLENGE`, `BLOB_CHALLENGE_RESPONSE`, `BLOB_ANNOUNCE` and their handlers.
- `src/services/slashing.js` — add offense type constants `BLOB_SERVE_FAILED`, `BLOB_COMMIT_UNHOSTED`.
- `src/mining/miner.js` — if `process.env.HONE_STORAGE_HOST=true`, start `hostRunner` alongside the inference handler.
- `src/index.js` — mount `blobRoutes` at `/api/blobs`.

**New ledger entry types**:
- `BLOB_STORE_COMMIT` — seller/deployer commits to having a blob hosted.
  Fields: `type`, `from` (payer), `to` (host username or `"hone_fs_pool"` if auto-assigned), `amount` (HONE paid), `epoch`, `blob_data: { cid, size_bytes, duration_epochs, host, fee_per_epoch, total_paid, escrow_id }`.
- `BLOB_SERVE_PROOF` — host reports "I am serving this CID as of epoch N". Fields: `type`, `from` (host), `epoch`, `blob_data: { cid, challenge_seed, chunk_index, chunk_hash, signature }`. Batch form carries `proofs: [...]`.
- `BLOB_CHALLENGE` — verifier challenges a host. Fields: `type`, `from` (verifier), `to` (host), `epoch`, `blob_data: { cid, chunk_index, nonce }`.
- `BLOB_SLASH` — derived entry written by slashing pipeline when a host fails a challenge; re-uses `slashing.executeSlash` machinery but adds a chain entry with `blob_data: { cid, reason }`.
- `BLOB_EXPIRE` — end of commitment: escrow releases remaining balance to host if probes passed, refunds payer fraction if probes failed. Fields: `blob_data: { cid, host, successful_epochs, total_epochs, final_payout, final_refund }`.

**New stateStore Maps + dispatcher cases**:
```
blobs: Map<cid, {
  cid, size_bytes, uploader, hosts: [{ host, committed_epoch, expires_epoch,
  fee_per_epoch, escrow_id, probes_passed, probes_failed, status }],
  first_committed_epoch, last_proof_epoch, status: 'committed'|'probing'|'expired'|'slashed'
}>
```
Dispatcher cases in `applyEntry`:
- `BLOB_STORE_COMMIT` — ensure blob record exists, append a host entry, do not charge balance here (the paired `ESCROW_LOCK` entry handles funds).
- `BLOB_SERVE_PROOF` — find the matching `{cid, host}` pair, bump `probes_passed`, update `last_proof_epoch`.
- `BLOB_CHALLENGE` — record the challenge as pending (for dedupe + response correlation); no money movement.
- `BLOB_SLASH` — mark host status `slashed`, reduce `probes_passed`, keep audit trail.
- `BLOB_EXPIRE` — mark the host's commitment expired; paired ESCROW_RELEASE and ESCROW_REFUND entries move money.

`_entryKey` gains a branch: `if (entry.blob_data && entry.blob_data.cid) domainId = "b:" + entry.blob_data.cid + ":" + (entry.blob_data.chunk_index || 0)`.

**New recordX functions in ledger.js** (all async):
- `recordBlobStoreCommit(payer, host, cid, sizeBytes, durationEpochs, feePerEpoch, escrowId, epoch)`
- `recordBlobServeProof(host, cid, challengeSeed, chunkIndex, chunkHash, signature, epoch)`
- `recordBlobChallenge(verifier, host, cid, chunkIndex, nonce, epoch)`
- `recordBlobExpire(cid, host, successfulEpochs, totalEpochs, finalPayout, finalRefund, epoch)`
- `commitBlobWithEscrow(payer, host, cid, sizeBytes, durationEpochs, feePerEpoch, epoch)` — convenience: generates `escrow_id`, calls `recordEscrowLock(payer, escrowId, totalFee, epoch)`, then `recordBlobStoreCommit`.

**HTTP routes added**:
- `POST /api/blobs` (JWT or API key) — multipart upload. Server computes CID, stores locally, returns `{ cid, size_bytes }`. Does **not** create a commit — upload and commit are separate steps.
- `GET /api/blobs/:cid` — public. Streams from local blob store; 404 if not here (caller can try another host).
- `POST /api/blobs/:cid/commit` (JWT) — body `{ host, duration_epochs, fee_per_epoch }`. Calls `commitBlobWithEscrow`. Returns 201 with `{ commit_id, escrow_id, total_paid }`. If `host === "auto"`, picks a registered storage host by reputation × capacity.
- `GET /api/blobs/:cid/status` — returns `stateStore.getBlob(cid)`.
- `GET /api/blobs/hosts` — lists known storage hosts (seller-visible directory).

**P2P messages added**:
- `BLOB_ANNOUNCE` — host broadcasts on startup: `{ host, blobs: [cid...], capacity_bytes, free_bytes }`. Any node can store in a local `blobDirectory` map for commit routing.
- `BLOB_CHALLENGE` — verifier → host: `{ cid, chunk_index, nonce }`. Also written as a ledger entry once the panel agrees on a challenge.
- `BLOB_CHALLENGE_RESPONSE` — host → verifier panel: `{ cid, chunk_index, chunk_bytes_hash, merkle_path, signature }`. Verifier panel cross-checks against the blob's known size + CID. On pass, verifier emits a `BLOB_SERVE_PROOF` entry; on fail, slashing pipeline records offense.

**Chain invariants**:
- `BLOB_STORE_COMMIT` requires a concurrent `ESCROW_LOCK` entry in the same block (enforced at route/ledger level, not dispatcher — dispatcher is accepting).
- `BLOB_SERVE_PROOF` is only valid if there's an existing `blobs[cid].hosts[host]` record with status `probing`. Drop otherwise.
- `BLOB_EXPIRE` may only be emitted once per `(cid, host)` pair — `_entryKey` dedupe with a "expire:" marker.
- Hosts cannot self-attest without a matching `BLOB_CHALLENGE` — `recordBlobServeProof` rejects if the nonce isn't in `seenChallenges`.
- Slashing on N consecutive failed challenges (N configurable, default 3) — triggers `slashing.executeSlash` with `offenseType: BLOB_SERVE_FAILED`.

**Reuse of existing code**:
- `services/escrow.js` untouched; called via `commitBlobWithEscrow`.
- `services/slashing.js` `executeSlash` + `recordOffense`.
- `inference/verifier.js` pattern for verifier panel selection (same 3-of-N logic).
- `p2p/protocol.js` `createMessage` + existing broadcast fan-out.
- `mining/miner.js` startup sequence (same `if (HONE_MINER) startInferenceHandler(); if (HONE_STORAGE_HOST) hostRunner.start();`).

**Tests to write** (`tests/honefs.test.js`):
- CID is deterministic (same bytes → same CID)
- CID collision: different bytes → different CID
- blobStore put/get roundtrip + listLocal
- blobStore get on missing → null
- ledger.commitBlobWithEscrow creates both ESCROW_LOCK and BLOB_STORE_COMMIT entries
- BLOB_SERVE_PROOF without prior BLOB_CHALLENGE is rejected at the verifier level (unit test of verifier.shouldAccept)
- BLOB_CHALLENGE + BLOB_SERVE_PROOF happy path updates `probes_passed`
- 3 consecutive failed challenges triggers slash (mock slashing.executeSlash)
- BLOB_EXPIRE after duration_epochs releases escrow to host
- BLOB_EXPIRE after N failures refunds a pro-rata fraction to payer
- finality snapshot round-trip includes blobs

**Verification steps**:
- `npm test` green
- Run two nodes locally, one with `HONE_STORAGE_HOST=true`, upload a 1MB file, commit for 10 epochs, confirm blob is served from both sides, kill the host mid-commitment, confirm slashing within 3 epochs.

**Rough effort**: large. (Biggest piece after v2.14.)

---

### Phase v2.12.x — Block size cap + fee market

**Goal**: cap block payloads at 1 MB and introduce a fee-per-byte mempool market that kicks in only when the block would overflow, keeping existing free transactions working in the common case.

**New files**:
- `src/p2p/feeMarket.js` — helpers: `estimateTxSize(tx)`, `getPriorityFee(tx)`, `sortByFeeRate(txs)`, `selectForBlock(txs, remainingBytes)`.
- `tests/blockCap.test.js` — overflow, selection, free-tx fallthrough, determinism.
- `tests/feeMarket.test.js` — fee-rate math unit tests.

**Modified files**:
- `src/chain/blockStore.js` — `writeBlock` accepts a `{ maxPayloadBytes = 1048576 }` option; throws `BlockPayloadTooLarge` if payload exceeds. Export `MAX_BLOCK_PAYLOAD = 1_048_576`. Read side unchanged.
- `src/mining/miner.js` — before serializing payload, call `feeMarket.selectForBlock(pendingEntries, MAX_BLOCK_PAYLOAD - reservedHeaderSpace)`; split selected vs. overflow; overflow stays in pending for next block; add two logs `[HONE] block bytes: X / Y, entries: M / N`. Add `getBlockSpaceRemaining()` helper exported from miner for introspection.
- `src/p2p/mempool.js` — `submit` accepts optional `tx.fee` (in dreams); tracks `feeRate = fee / estimatedBytes`; new `getSortedByFeeRate()` iterator; new `getStats()` field `congested: boolean` (true when mempool > 80% full). Free (fee=0) transactions still accepted but sorted last.
- `src/services/ledger.js` — commerce + blob + service recordX take an optional `fee = 0` argument, attached to the entry as `entry.fee`. No behavior change when zero.

**New ledger entry types**: none. This is a serialization/scheduling change, not a semantic one.

**New stateStore Maps + dispatcher cases**: none. `entry.fee` is just a field.

**New recordX functions in ledger.js**: none. Existing ones gain an optional `fee` kwarg.

**HTTP routes or P2P messages added**:
- `GET /api/mempool/stats` (public) — returns `{ size, maxSize, congested, minFeeRate, recommendedFeeRate, freeEntriesCount }`. Wallet UIs hit this to decide whether to attach a fee.
- No new P2P messages. Existing `TRANSACTION` broadcast already carries the whole tx object including `fee` once added.

**Chain invariants**:
- Block payload bytes ≤ `MAX_BLOCK_PAYLOAD` (1,048,576). Hard-enforced in `writeBlock`.
- Merkle roots of `ledger_entries` must match the actually-included subset — overflow entries are re-serialized into the next epoch with a fresh `timestamp` or retained as-is (decision flagged in section 5 open questions).
- Existing free transactions MUST remain valid when block is under cap. Fee is additive, not required.
- Fee payments go to `hone_treasury` as a `TRANSFER` entry appended implicitly when the tx is selected (not at submit time — only applied on inclusion to avoid charging for dropped txs). Implementation: `feeMarket.selectForBlock` returns `{ selected, feeEntries }` and miner appends `feeEntries` to `epochLedgerEntries` before writing.

**Reuse of existing code**:
- `p2p/mempool.js` `pending` Map stays, new iterator is added on top.
- `chain/blockStore.js` payload shape unchanged; only adds a size guard.
- `mining/miner.js` existing `flushPendingEntries` flow; just adds the selection step before writeBlock.
- Existing commerce tests stay green because `fee` defaults to zero.

**Tests to write**:
- `tests/blockCap.test.js`:
  - `writeBlock` rejects a payload > 1 MB with `BlockPayloadTooLarge`
  - miner with 2000 pending entries under 1 MB writes them all
  - miner with 2000 pending entries over 1 MB writes a subset, retains rest
  - determinism: same input set + fees → same subset chosen (fee-rate tiebreak by txHash)
  - free transactions still included when there's space
  - fee transactions outrank free transactions when there isn't
- `tests/feeMarket.test.js`:
  - `estimateTxSize` roughly matches JSON.stringify length
  - `sortByFeeRate` puts zero-fee last, ties broken by hash
  - `selectForBlock` returns exactly as many entries as fit

**Verification steps**:
- `npm test` — all existing tests still pass (no fee changes anywhere else means commerce.test.js, ledger.test.js, honefs.test.js stay green)
- Manual: seed mempool with 10k tiny transfers, mine a block, confirm block is ≤ 1 MB and log shows overflow count
- Manual: run smoke flow — confirm fee-less TRANSFER still works when mempool is small

**Rough effort**: medium.

---

### Phase v2.13.x — SERVICE_DEPLOY (stateless compute hosting)

**Goal**: let deployers publish a runtime spec (pointing at a v2.11 CID binary) and let miners opt in to hosting it; users pay per session via escrow; heartbeats prove liveness.

**Planned sub-versions**:
- `v2.13.0` — ledger entries, stateStore maps, HTTP routes for deploy/session
- `v2.13.1` — host-side `serviceHost.js` subprocess wrapper (Docker/Podman)
- `v2.13.2` — P2P `SERVICE_HEARTBEAT` + verifier uptime probes + auto-reputation
- `v2.13.3` — session pro-rata settlement + stale-session sweep

**New files**:
- `src/services/serviceHost.js` — host-side runner. Given a SERVICE_DEPLOY ledger entry we've opted into, pulls the binary via HONE-FS, runs `podman run --rm -d --memory=Xm --cpus=Y -p <port> --entrypoint <...> <imagish>`, tracks process handle, publishes SERVICE_HEARTBEAT each epoch, tears down on session end. Uses subprocess — does not reinvent orchestration.
- `src/services/serviceSpec.js` — parses + validates `runtime` field: `{ type: "http"|"tcp"|"wasm", cpu, ram_mb, gpu?, ports, entrypoint, binary_cid, env_whitelist }`. Rejects host path escapes, privileged flags, unrestricted network.
- `src/routes/serviceRoutes.js` — `POST /api/services` (deploy), `GET /api/services` (list), `GET /api/services/:id` (detail), `POST /api/services/:id/sessions` (start), `DELETE /api/sessions/:id` (end).
- `src/p2p/serviceHandler.js` — P2P handler for SERVICE_HEARTBEAT gossip + SERVICE_PROBE challenge.
- `tests/serviceHost.test.js`, `tests/serviceRoutes.test.js`.

**Modified files**:
- `src/chain/stateStore.js` — add `services` and `sessions` Maps; dispatcher cases; getters; snapshot; resetAll; hydrate; `_entryKey`.
- `src/services/ledger.js` — new recordX functions (see below).
- `src/chain/stateManager.js` — include `services`, `sessions` in finality snapshot.
- `src/p2p/protocol.js` — `SERVICE_HEARTBEAT`, `SERVICE_PROBE`, `SERVICE_PROBE_RESPONSE` message types.
- `src/services/slashing.js` — add `SERVICE_DOWNTIME` offense type.
- `src/mining/miner.js` — if `process.env.HONE_COMPUTE_HOST=true`, start `serviceHost` loop.
- `src/index.js` — mount `serviceRoutes`.

**New ledger entry types**:
- `SERVICE_DEPLOY` — deployer publishes a service. Fields: `type`, `from` (deployer), `epoch`, `service_data: { service_id, name, runtime, price_per_second, accepted_token, max_sessions, binary_cid, min_host_rep, created_epoch }`. Optional extension: `product_id` to link to an existing PRODUCT_CREATE.
- `SERVICE_OFFER` — a miner opts in to host. Fields: `from` (host), `service_data: { service_id, host, committed_until_epoch, capacity }`.
- `SERVICE_HEARTBEAT` — host proves uptime. Fields: `from` (host), `epoch`, `service_data: { service_id, host, active_sessions, state_commitment, signature }`.
- `SERVICE_PROBE_RESULT` — verifier reports probe pass/fail. Fields: `service_data: { service_id, host, verifier, result, latency_ms, epoch }`.
- `SESSION_START` — user begins a session. Fields: `from` (user), `to` (host), `amount` (escrow max), `epoch`, `session_data: { session_id, service_id, host, max_duration_sec, max_total_cost, escrow_id, start_epoch }`.
- `SESSION_END` — settlement. Fields: `from` (user or host or system sweep), `session_data: { session_id, end_epoch, actual_duration_sec, final_cost, final_refund, reason: 'user_end'|'host_end'|'timeout'|'slash' }`.

**New stateStore Maps + dispatcher cases**:
```
services: Map<service_id, {
  service_id, deployer, name, runtime, price_per_second, accepted_token,
  max_sessions, binary_cid, min_host_rep, created_epoch,
  hosts: [{ host, committed_until_epoch, capacity,
            last_heartbeat_epoch, probes_passed, probes_failed, status }],
  active_sessions: Set<session_id>, total_sessions: number, status
}>

sessions: Map<session_id, {
  session_id, service_id, user, host, max_duration_sec, max_total_cost,
  escrow_id, start_epoch, end_epoch, actual_duration_sec, final_cost,
  final_refund, status: 'active'|'ended'|'timed_out'|'slashed'
}>
```

Dispatcher cases: straightforward writes; `SESSION_END` updates the service's `active_sessions` set and runs settlement math; `SERVICE_HEARTBEAT` updates `last_heartbeat_epoch` only if the host is a member of `services[id].hosts`.

`_entryKey`: add branches for `service_data.service_id` and `session_data.session_id`.

**New recordX functions in ledger.js**:
- `recordServiceDeploy(deployer, serviceData, epoch)` — caller must already have paid a deploy fee via TRANSFER; invariant checked at route level.
- `recordServiceOffer(host, serviceId, committedUntilEpoch, capacity, epoch)`
- `recordServiceHeartbeat(host, serviceId, activeSessions, stateCommitment, signature, epoch)`
- `recordServiceProbeResult(verifier, serviceId, host, result, latencyMs, epoch)`
- `recordSessionStart(user, host, serviceId, maxDurationSec, maxTotalCost, epoch)` — generates `session_id`, calls `recordEscrowLock(user, 'session:'+id, maxTotalCost, epoch)`.
- `recordSessionEnd(party, sessionId, actualDurationSec, reason, epoch)` — computes `final_cost = min(maxTotalCost, actualDurationSec * price_per_second)`, calls `recordEscrowRelease(host, ...)` for `final_cost` and `recordEscrowRefund(user, ...)` for the difference.

**HTTP routes added** (under `/api/services`):
- `POST /` (JWT) — deploy. Body: `{ name, runtime, price_per_second, accepted_token, max_sessions, binary_cid, min_host_rep }`. Validates `runtime` via `serviceSpec.validate`.
- `GET /` — browse, filter by `status`, `type`, `min_host_rep`.
- `GET /:id` — detail including host list.
- `POST /:id/offer` (JWT, host) — opt in.
- `POST /:id/sessions` (JWT) — start a session. Body: `{ host, max_duration_sec, max_total_cost }`. Returns `{ session_id, escrow_id, host_url }` where `host_url` is resolved via P2P directory.
- `DELETE /api/sessions/:id` (JWT, user or host) — end a session.
- `GET /api/sessions/me` (JWT) — my sessions.

**P2P messages added**:
- `SERVICE_HEARTBEAT` — gossiped every epoch. Ledger entry form of the same.
- `SERVICE_PROBE` — verifier → host: `{ service_id, nonce, expected_capability }`. Host is expected to respond over the service's declared port with a signed response within a timeout.
- `SERVICE_PROBE_RESPONSE` — verifier panel collects; 2-of-3 agreement writes a `SERVICE_PROBE_RESULT` entry.

**Chain invariants**:
- `SERVICE_DEPLOY.runtime.binary_cid` must resolve to an existing `blobs[cid]` with at least one active host (enforced by dispatcher; v2.11 dependency).
- `SERVICE_OFFER` requires the host to have `min_host_rep` reputation score AND not be slashed.
- `SESSION_START` requires the target host to be in `services[id].hosts` with `last_heartbeat_epoch` within the last N epochs (liveness window, default 3).
- `SESSION_END` may only be emitted once per session; dispatcher dedupes.
- `final_cost + final_refund == escrow.amount` (pro-rata math is deterministic and verified in dispatcher).
- N consecutive failed SERVICE_PROBE_RESULTs slash the host.
- Heartbeat gap > M epochs marks host as `inactive` — new sessions rejected.

**Reuse of existing code**:
- `honefs/*` — binary distribution and integrity.
- `services/escrow.js` — session escrows.
- `inference/verifier.js` — probe panel formation.
- `p2p/protocol.js` broadcast.
- `services/slashing.js`.
- Commerce product/reputation Maps — SERVICE_DEPLOY can link to a PRODUCT_CREATE so users browse compute services through the commerce UI.

**Tests to write**:
- `tests/serviceRoutes.test.js`: deploy with invalid runtime (400), deploy with missing binary CID (409), deploy happy path, offer flow, session start/end happy path, session auto-end on timeout, session end refunds unused escrow.
- `tests/serviceHost.test.js`: mock subprocess wrapper; verify env whitelist, verify port binding, verify teardown on signal.
- Uptime probe path: mock verifier panel, simulate success + failure, assert reputation delta.

**Verification steps**:
- Start 3 nodes: deployer, host (with `HONE_COMPUTE_HOST=true`), user.
- Deployer uploads a simple HTTP echo binary to HONE-FS, commits 100 epochs.
- Deployer calls `POST /api/services` with the CID.
- Host calls `POST /api/services/:id/offer`.
- User calls `POST /api/services/:id/sessions`, gets back `host_url`, confirms echo works.
- Kill host mid-session, confirm session refunds and reputation drops.

**Rough effort**: large.

---

### Phase v2.14.x — Stateful compute with snapshot replication

**Goal**: extend v2.13 with persistent state via periodic snapshots uploaded to HONE-FS; on host failure, the next scheduled host fetches the latest snapshot CID and resumes.

**Planned sub-versions**:
- `v2.14.0` — STATE_SNAPSHOT ledger entry + snapshot runner
- `v2.14.1` — STATE_TRANSFER handoff protocol
- `v2.14.2` — state-verification challenges + slashing hookup
- `v2.14.3` — optional WAL support (flagged as open question)

**New files**:
- `src/services/stateSnapshot.js` — host-side: on a timer (`snapshot_interval_sec` from runtime), serialize in-memory state (callback defined per-service by the runtime — the chain doesn't interpret the bytes), gzip, put into local blobStore, record STATE_SNAPSHOT.
- `src/services/stateTransfer.js` — replacement-host-side: on being elected, fetch latest STATE_SNAPSHOT CID for the service, pull bytes via honefs, hand to the service runtime, emit a `STATE_TRANSFER` entry when ready.
- `src/services/stateVerifier.js` — verifier: samples random keys from a snapshot (via a host-provided key index), challenges the current host to return those keys' values within a timeout.
- `tests/stateful.test.js`.

**Modified files**:
- `src/chain/stateStore.js` — extend `services[id]` with `latest_snapshot: { cid, epoch, host }` and a new Map `snapshots: Map<service_id, Array<{cid, epoch, host, size_bytes}>>`. Add dispatcher cases, getters, snapshot/hydrate/resetAll/_entryKey.
- `src/services/ledger.js` — new recordX functions.
- `src/chain/stateManager.js` — include `snapshots` + `latest_snapshot` in finality snapshot.
- `src/services/serviceSpec.js` — extend schema: `snapshot_interval_sec`, `snapshot_keyspace: ['foo', 'bar']` (for spot-check challenges), `handoff_grace_epochs`.
- `src/services/serviceHost.js` — integrate `stateSnapshot.run(serviceId, intervalSec)` for services that declare `snapshot_interval_sec > 0`.
- `src/p2p/protocol.js` — `STATE_VERIFY`, `STATE_VERIFY_RESPONSE` message types.

**New ledger entry types**:
- `STATE_SNAPSHOT` — host uploads a snapshot. Fields: `from` (host), `epoch`, `state_data: { service_id, cid, size_bytes, key_count, key_sample_root, snapshot_epoch, signature }`.
- `STATE_TRANSFER` — handoff. Fields: `from` (new host), `to` (old host), `epoch`, `state_data: { service_id, source_snapshot_cid, source_snapshot_epoch, reason: 'host_failure'|'planned_handoff', signature }`.
- `STATE_VERIFY_RESULT` — verifier outcome. Fields: `state_data: { service_id, host, snapshot_cid, sampled_keys, result: 'pass'|'fail', failed_keys, epoch }`.

**New stateStore Maps + dispatcher cases**:
```
snapshots: Map<service_id, [{
  cid, host, epoch, size_bytes, key_count, key_sample_root,
  verified: 'pending'|'passed'|'failed'
}]>  // sorted newest first, capped at last 10 per service
```
Dispatcher:
- `STATE_SNAPSHOT` — append to `snapshots[service_id]`; update `services[service_id].latest_snapshot`; trim history to N.
- `STATE_TRANSFER` — validate: new host must be a current member of `services[id].hosts`; old host's `committed_until_epoch` must have passed OR old host must be marked `inactive`; rewrite `services[id].hosts[*].status` accordingly; emit handoff event.
- `STATE_VERIFY_RESULT` — update the referenced snapshot's `verified` field; on `fail`, trigger slashing of the host that produced the snapshot.

`_entryKey` gains: `"ss:" + service_id + ":" + snapshot_epoch` and similar for transfers.

**New recordX functions in ledger.js**:
- `recordStateSnapshot(host, serviceId, cid, sizeBytes, keyCount, keySampleRoot, signature, epoch)`
- `recordStateTransfer(newHost, oldHost, serviceId, sourceSnapshotCid, sourceSnapshotEpoch, reason, signature, epoch)`
- `recordStateVerifyResult(serviceId, host, snapshotCid, sampledKeys, result, failedKeys, epoch)`

**HTTP routes / P2P messages added**:
- No new user-facing HTTP routes (runs on miners). Optional read endpoint: `GET /api/services/:id/snapshots` (list) and `GET /api/services/:id/snapshots/latest` (for debugging).
- P2P: `STATE_VERIFY` verifier → host (`{ service_id, snapshot_cid, sampled_keys, nonce }`); `STATE_VERIFY_RESPONSE` host → verifier (`{ keys: [{key, value_hash}], signature }`).

**Chain invariants**:
- `STATE_SNAPSHOT.cid` must already exist in `blobs` (host committed it via v2.11). If not, dispatcher drops. Blobs committed by STATE_SNAPSHOT are special: their duration is a function of the snapshot retention policy, not a free-form host bid.
- `STATE_TRANSFER.source_snapshot_cid` must be the latest passed snapshot for the service (from stateStore).
- Only one active handoff per service at a time — second transfer in same epoch drops.
- Failed `STATE_VERIFY_RESULT` slashes host AND invalidates the snapshot; next handoff will pick the previous-passed snapshot.
- Acceptance: "in-flight writes between snapshots are lost unless a WAL is configured." This is an explicit property — dispatcher does not attempt to replay anything between `latest_snapshot.epoch` and failure.

**Reuse of existing code**:
- `services/serviceHost.js` (v2.13) host loop — extended with snapshot timer.
- `honefs/*` — snapshot storage.
- `inference/verifier.js` panel formation.
- `services/slashing.js`.
- `services/escrow.js` — per-snapshot storage escrow reuses v2.11 flow.

**Tests to write** (`tests/stateful.test.js`):
- STATE_SNAPSHOT without prior blob commit → rejected
- STATE_SNAPSHOT happy path updates `services[id].latest_snapshot`
- snapshot history is capped at 10
- STATE_TRANSFER from non-member host → rejected
- STATE_TRANSFER uses latest PASSED snapshot
- STATE_VERIFY_RESULT fail triggers slashing
- finality hydration restores services[*].latest_snapshot
- determinism: same snapshot history → same stateStore.snapshot() output

**Verification steps**:
- 3-node integration: deploy a stateful counter service, increment it N times, kill the host, confirm the new host resumes from the latest snapshot with counter value close to N (up to 1 snapshot_interval loss), confirm slashing if the new host lies about a sampled key.

**Rough effort**: very large. (Hardest phase. See risks in §5.)

---

## 3. Cross-cutting concerns

### 3a. Finality snapshot extensions

The v2.10.0 codebase has a latent bug: `stateManager.generateFinalitySnapshot().extended_state` only emits `tokens`, `projects`, `extra_balances`. Meanwhile `stateStore.hydrateFromFinality` already reads `stores`, `products`, `orders`, `reputation`, `reputation_votes`, `delegations`. **v2.10.1 must fix this as step 1**, otherwise a finality-block restart loses all commerce state. Every subsequent phase adds its new Maps here too:

- v2.10.1: stores, products, orders, reputation, reputation_votes, delegations (closes gap)
- v2.11: blobs
- v2.13: services, sessions
- v2.14: snapshots (capped history) and mirrors of services[*].latest_snapshot

The `hydrateFromFinality` function should be updated in the same commit as each phase ships — one `if (ext.blobs) { ... }` block per Map.

### 3b. `_entryKey` dedupe additions

Every phase adds a domainId branch so multiple entries of the same type per (epoch, sender) don't collide:

- v2.11: `if (entry.blob_data) domainId = "b:" + cid + ":" + (chunk_index||0) + ":" + (expire?'e':'')`
- v2.13: `if (entry.service_data) domainId = "svc:" + service_id + ":" + (host||'') + ":" + (heartbeat_epoch||epoch)`; `if (entry.session_data) domainId = "sess:" + session_id + ":" + (end_epoch||'start')`
- v2.14: `if (entry.state_data) domainId = "st:" + service_id + ":" + (snapshot_epoch||transfer_epoch||'')`

Pattern: append a unique identifier for every Map-mutating entry type to keep `seenEntries` correct across gossip replay.

### 3c. Security concerns

- **Auth**: JWT for end-user routes, API key (`authenticateApiKey`) for integration callers. No route should accept both identities for the same mutation except where it's explicit (sellers operating via bots).
- **Authorization**: ownership checks at route level: "only seller X can fulfill order O", "only host H can heartbeat service S". Dispatcher already enforces some of this silently; routes should fail loudly with 403.
- **Slashing triggers** (new in v2.11–v2.14): blob serve failures, service downtime, state snapshot mismatch, lying in STATE_VERIFY_RESPONSE. All flow through existing `slashing.executeSlash`, which is already chain-recorded.
- **DoS vectors**:
  - v2.10.1: spam store open → mitigated by required stake (already in chain). Spam product creation → mitigated by capacity cap.
  - v2.11: spam blob upload → mitigated by requiring a commit with escrow before anything is hosted beyond a local 10MB ephemeral buffer. Spam BLOB_CHALLENGE → rate-limit verifier panel participation.
  - v2.12: spam mempool → already has MAX_MEMPOOL_SIZE=1000; fee market raises the cost of spamming.
  - v2.13: spam service deploy → require a non-zero deploy fee (paid to treasury, tunable). Spam session start → per-user session cap, bonded by user balance.
  - v2.14: spam snapshots → snapshot interval is declared in service spec and enforced by dispatcher (reject STATE_SNAPSHOT if last one was < interval_sec ago).
- **Sandboxing (v2.13+)**: `serviceSpec.validate` must reject `--privileged`, `--network=host`, volume mounts outside a chroot, capability adds. `podman run --rm --read-only --tmpfs /tmp` is the baseline.
- **Secret handling**: service env vars go through an `env_whitelist` in the runtime spec. No shell interpolation. No host-side secret injection without opt-in.
- **Signature verification**: SERVICE_HEARTBEAT, BLOB_SERVE_PROOF, STATE_SNAPSHOT, STATE_TRANSFER all carry signatures. Verified against the sender's registered public key before `applyEntry` accepts. Currently the codebase uses opportunistic verification — we need a helper `ledger.verifyEntrySignature(entry)` to formalize this, called at the dispatcher boundary for these new types.

### 3d. Migration concerns

Running chain is v2.10.0 with live commerce state. Migration rules:

- v2.10.1: pure additive (new routes, finality snapshot fix, wrapper functions). Zero migration. Fix in finality snapshot is backward-compatible since `hydrateFromFinality` already guarded with `if (ext.stores)`.
- v2.11.0: additive. New ledger types are skipped by older nodes (`default` case in dispatcher). Older nodes would miss blob events until upgraded — document as a soft fork: all nodes must upgrade to v2.11 before any BLOB entries are emitted on the live chain. The chain will converge regardless but blob state won't be visible on pre-v2.11 nodes.
- v2.12.0: **semi-breaking**. Adds 1 MB cap to `writeBlock`. Any node emitting a block > 1 MB will fail to write. Ship v2.12 as "soft-disabled" at first: `writeBlock` logs a warning if payload > 1 MB but only throws when `HONE_ENFORCE_BLOCK_CAP=true`. Flip the default to strict in v2.12.1 after observation period.
- v2.13, v2.14: additive, same soft-fork pattern.
- **Never reintroduce Mongo writes for chain state**. The v2.10.1 routes must use `stateStore.get*` for reads and `ledger.recordX` for writes, never `new Project(...)`-style Mongoose writes for commerce entities. Mongoose is still used for auth (`User`, `Wallet.address` for lookup) — that's fine, those aren't chain state.
- Finality snapshot format change in v2.10.1 requires a one-time replay to repopulate `extended_state.stores` etc. on nodes that started from a v2.10.0 finality block. Document as "stop miner, delete `data/blocks/finality-*.bin` newer than the commerce-using epoch, restart, replay from blocks." Alternative: stateStore also reads from in-between block files so this happens automatically — verify by running `replay.js` on a v2.10.0 snapshot.

### 3e. Testing strategy

- **Unit**: per-function jest tests for new ledger recordX, stateStore dispatcher cases, CID math, fee market math, bonding curve already covered.
- **Integration**: per-phase route tests with supertest hitting a fresh Express app, using mocked mempool + stateStore.resetAll() in `beforeEach`.
- **E2E**: new `tests/e2e/` or `scripts/smoke-*.sh` for each phase, running a full 2- or 3-node local network. v2.11 and v2.13 absolutely need E2E because P2P challenge flows can't be fully unit-tested.
- **Determinism**: every phase should add a "same entries → same snapshot hash" test to guard against nondeterministic Map iteration leaking into block serialization.
- **Regression**: each phase adds a tiny test that runs the previous phase's happy path end-to-end, to catch unintended cross-phase breakage.

---

## 4. Dependencies and ordering

```
v2.10.1 ──────┬──────────── v2.12  (block cap / fees — independent)
              │
              └─→ v2.11 ─┬──→ v2.13 ─→ v2.14
                         │
                         └──→ (v2.13 uses v2.11 for binaries;
                               v2.14 uses v2.11 for snapshots)
```

Hard dependencies:
- v2.11 requires v2.10.1 (routes pattern, finality fix)
- v2.13 requires v2.11 (binary_cid resolves to blobs) and v2.10.1 (reputation tie-in)
- v2.14 requires v2.13 (services Map) and v2.11 (snapshot CIDs)

Parallelizable work:
- **v2.12 is independent of v2.11/2.13/2.14** and can be developed alongside v2.11 on a separate branch. It touches mempool + blockStore + miner only; no conflicts with the commerce/fs/service code.
- Within v2.11: CID helper + blobStore (v2.11.0) can be built while someone else drafts the challenge protocol (v2.11.1).
- Within v2.13: `serviceSpec.js` + routes (v2.13.0) can be built while someone else drafts `serviceHost.js` subprocess wrapper (v2.13.1).
- Test writing in every phase is parallelizable with the implementation itself once the ledger entry shape is agreed.

Recommended build order for a single-person sequential implementation:
1. v2.10.1 (closes the finality gap, unblocks everything).
2. v2.12.0 (fast win, no dependencies, gets the chain discipline in before bigger changes land).
3. v2.11.0 → v2.11.1 → v2.11.2 → v2.11.3.
4. v2.13.0 → v2.13.1 → v2.13.2 → v2.13.3.
5. v2.14.0 → v2.14.1 → v2.14.2. v2.14.3 (WAL) only if the user says they need it.

---

## 5. Known risks and open questions

These should be discussed with the user before building, not decided unilaterally.

1. **Escrow release timing for orders (v2.10.1).** Does the seller get paid on ORDER_FULFILL, on ORDER_DELIVERED, or on `autoRelease` after N days? Three different trust models. Current v2.10 ledger allows any of them. Recommend: pay on DELIVERED with auto-release 7 days after FULFILL (matches eBay/Amazon). Needs user confirmation.

2. **Reputation weight formula (v2.10.1).** v2.10 lets the caller pass any weight 1–100; spec says "determined by caller (stake/completed txn count)" but nothing enforces it. Route layer needs a formula. Propose: `weight = min(100, floor(sqrt(completed_orders_with_target) * 5) + floor(sqrt(stake) * 2))`. Needs sign-off.

3. **Stable token identity (v2.10.1).** `wUSDC`, `wUSDT`, `wDAI` are referenced but I didn't see where these wrapper tokens actually get minted / bridged. Does recording `STAKE_PURCHASE` with `token: 'wUSDC'` require the caller to actually hold that token, or is this aspirational? Affects whether route should validate balance.

4. **Host assignment strategy (v2.11).** When a deployer commits a blob with `host: "auto"`, what picks the host? Reputation? Lowest fee? Random stake-weighted? A simple reputation-weighted random ballot per commit is defensible — needs approval.

5. **HONE-FS payment unit (v2.11).** Is storage priced in HONE or in a stable? Mixing tokens across phases is fine technically, but the UX suffers if a user has to hold 5 different tokens to use the chain. Recommend: HONE only for hosting fees; stable only for store-capacity bonding curve (already the case).

6. **Challenge frequency (v2.11).** Probe every epoch is overkill for small blobs and expensive for large ones. Size-weighted probe interval? Random sampling so only M% of blobs are probed each epoch? Needs a concrete policy.

7. **Block cap rollout (v2.12).** Soft warning first, or hard fail immediately? See §3d. Related: what happens to a SERVICE_HEARTBEAT that a verifier tries to emit into an already-full block? Drop silently, retry next epoch, or reserve a priority lane for system entries (heartbeats, mining proofs)? Recommend: reserve ~20% of block for system entries that aren't fee-paying.

8. **Merkle root + overflow determinism (v2.12).** If block N can't fit all pending entries, and entries are deferred to block N+1, the merkle root of block N is over the selected subset — determinism requires every node to make the same selection. Propose: all nodes apply the same deterministic selection algorithm (sort by fee-rate desc, break ties by ledger-entry canonical hash asc). But this assumes every node sees the same pending set, which the gossip protocol doesn't fully guarantee. This is **the biggest correctness risk in v2.12** and deserves its own design doc before coding.

9. **Subprocess lifecycle on host crash (v2.13).** If the miner process dies mid-session, podman containers stay orphaned. Options: (a) use systemd-run with --scope tied to the miner pid, (b) use a watchdog process, (c) accept leakage and clean up at startup by listing all `hone-service-*` containers and pruning. Recommend (c) as MVP.

10. **State snapshot semantics (v2.14).** "State" is whatever the service implementation writes to disk at snapshot time. The chain has no opinion on the format. But the sampled-key challenge protocol requires the host to produce `key → value_hash` for arbitrary keys. That means the service runtime has to expose a `GET /_hone/state/:key` introspection endpoint. **This is a real coupling between the service and the chain** — the chain can't verify blobs it doesn't understand. Options:
    - (a) Require all stateful services to implement a standard introspection endpoint (forces a runtime SDK, cleaner long-term).
    - (b) Only verify snapshot size + CID, not content (weaker — lets hosts ship garbage). User likely wants (a).
11. **Lost in-flight writes (v2.14).** Explicitly documented as acceptable. Is there user demand for a WAL (v2.14.3)? If yes, WAL entries become a new ledger type STATE_WAL_APPEND, pushed between snapshots. That doubles the storage cost and may not be worth it — probably defer.
12. **Recovery from total host failure (v2.14).** If every host holding snapshots for a service is slashed simultaneously, the service is unrecoverable. Acceptable? Or do we need a "cold storage host" tier that's paid less but replicates everything?
13. **Backpressure in replay.** v2.11–v2.14 dispatcher cases mutate larger Maps. On a long replay (10k+ blocks), Map grow+iteration costs add up. Benchmark replay on a synthetic chain before shipping v2.14. Not a blocker, but a known risk.

---

## 6. Deferred / out of scope

The user asked for these five phases and nothing else. The following items sound related but are explicitly NOT part of this plan:

- **Governance system**. No on-chain voting, no parameter-change proposals, no treasury governance. Bonding curve params are constants in code.
- **Tokenomics changes**. Supply, emission schedule, reward splits — all unchanged. No new minting mechanisms.
- **Bridge integrations**. Hive, TON, Ethereum bridges stay as they are. Wrapped stables (wUSDC/wUSDT/wDAI) are assumed to already exist or be implementable separately.
- **Frontend / native UI**. The phases produce routes; the Telegram bots and website can consume them. No new web UI beyond keeping existing routes usable.
- **Dream ecosystem changes**. Dreams, GenesisDream, revenue-share NFTs — touched only where they already intersect (e.g. reputation voting is generic). No new dream types.
- **Mongoose model cleanup (Phase E/F on the task tracker)**. Separate workstream — these phases coexist with the Mongo-backed auth and wallet cache, and do not block on Phase E completing.
- **Genesis re-inscription**. No changes to genesisBlock.js or the reserved premium name list.
- **Cross-chain claims / bridge proof changes**. The claim generator in `src/claims/` stays untouched.
- **NFT feature expansion**. Existing composable/rental/evolving NFTs stay as-is; no new NFT types added in these phases.
- **Automated dispute resolution**. Dispute appeals continue through the existing `appealRoutes.js` + panel flow. v2.10.1 only adds the dispute endpoint; arbitration logic is unchanged.
- **Global content moderation**. `services/contentFilter.js` stays as-is; commerce routes do NOT add a new moderation layer.
- **Smart contract VM**. v2.13 `runtime: "wasm"` is listed in the service spec but not implemented — WASM hosting is a stretch goal deferred to "v2.13 later" or its own phase. MVP is `type: "http"` and `type: "tcp"` via podman.
- **Light clients**. Chain stays full-node-only. No SPV, no pruned-archival split beyond existing Lucid Pruning.

---

### Critical Files for Implementation

The files most critical to read and modify across all five phases:

- /home/ubuntclaw/repos/hone/src/chain/stateStore.js
- /home/ubuntclaw/repos/hone/src/services/ledger.js
- /home/ubuntclaw/repos/hone/src/chain/stateManager.js
- /home/ubuntclaw/repos/hone/src/chain/blockStore.js
- /home/ubuntclaw/repos/hone/src/mining/miner.js

Secondary but load-bearing:
- /home/ubuntclaw/repos/hone/src/p2p/protocol.js
- /home/ubuntclaw/repos/hone/src/p2p/mempool.js
- /home/ubuntclaw/repos/hone/src/services/escrow.js
- /home/ubuntclaw/repos/hone/src/routes/projectRoutes.js (pattern reference)
- /home/ubuntclaw/repos/hone/tests/commerce.test.js (test pattern reference)
---

# Plan revision — v2.10.2 Gateway (discoverability layer)

Inserted between v2.10.1 and v2.11.x on user approval (2026-04-10). Answers the question "how does anyone actually reach a store or a hosted service?" Without this phase, everything on chain is reachable only via direct API calls with full CIDs.

## Goal
`honemesh.net/stores/<seller>/<slug>` and `honemesh.net/s/<slug>` render real pages from stateStore. Same gateway codebase grows to handle `/fs/<cid>` (v2.11) and `/service/<slug>` (v2.13) as those phases land.

## New files

- `src/gateway/server.js` — HTTP server, route registration, starts on GATEWAY_PORT (default 4343, falls back if taken). Optional; not started in the default node process unless `HONE_GATEWAY_ENABLED=true` or explicitly required.
- `src/gateway/routes.js` — gateway-specific routes (kept separate from `src/routes/commerceRoutes.js` which is the API)
- `src/gateway/storefront.js` — server-side HTML rendering for stores and products. Template-literal based, no framework dep. Reads from stateStore.
- `src/gateway/resolver.js` — turns a path (`/stores/<seller>/<slug>`) into a stateStore lookup. Single source of resolution logic, reused across storefront + future blob + service routes.
- `src/gateway/remoteClient.js` — optional lightweight mode: query a remote HONE node via JSON-RPC instead of local stateStore. Gated on `HONE_GATEWAY_REMOTE_RPC_URL` env var.
- `tests/gateway.test.js` — unit tests for resolver, routes, storefront rendering

## Modified files

- `src/index.js` — conditionally start the gateway server when env var is set. Do not bundle with default node startup to keep the core API unchanged.
- `bin/hone-gateway` — new CLI entry for standalone gateway (optional convenience script)

## Routes served (v2.10.2 only — blob and service routes arrive in later phases)

- `GET  /` — landing page listing recently created stores
- `GET  /stores` — paginated directory of all active stores
- `GET  /stores/:seller` — seller profile page (store metadata + product grid)
- `GET  /stores/:seller/:slug` — single product page (title, price, rep score, stock, description, buy button → posts to commerce API)
- `GET  /s/:shortcode` — short-URL form that redirects to `/stores/:seller/:slug` (shortcode = sha256(seller|slug) first 6 chars, deterministic, no separate index needed if we just store the short form alongside the product)
- `GET  /api/resolve/:path` — JSON resolver endpoint for programmatic use
- `GET  /health` — gateway health check

All gateway routes are read-only. Any "buy" button on a rendered page posts to the commerce API routes from v2.10.1 (via fetch from the rendered HTML), not to the gateway itself. This keeps gateway code small and auditable.

## Slug format decision (locked in)

`product_id` becomes `<seller>/<slug>` where:
- `<seller>` is the account name that created the product
- `<slug>` is user-chosen, must match `^[a-z0-9][a-z0-9-]{0,62}$` (lowercase, digits, dashes; starts alphanumeric; max 63 chars)
- Unique per seller — chain invariant enforced in stateStore.PRODUCT_CREATE dispatcher
- Global uniqueness is automatic because `<seller>/<slug>` is the primary key

Enforce in v2.10.1 at the route layer (return 400 on bad format) AND in stateStore (silently drop invalid entries on dispatch) — defense in depth.

## Chain invariants added this phase

None (routes are read-only). The slug format invariant lands in v2.10.1 alongside the route layer.

## Reuse of existing code

- `stateStore.getStore`, `getAllStores`, `getProduct`, `getAllProducts`, `getReputation`, `getOrdersBySeller` — already exist, gateway just reads them
- `src/services/ledger.js` — gateway never calls recordX; it's a pure reader
- Existing express app pattern from `src/index.js` — gateway reuses express
- Content-Security-Policy + basic security headers from existing middleware (if any)

## Tests

- `tests/gateway.test.js`:
  - resolver parses `/stores/alice/widget` → { seller: 'alice', slug: 'widget' }
  - resolver rejects `/stores/BADNAME/slug` (uppercase in seller)
  - storefront renders a product page with title + price + rep
  - `/stores/:seller` 404s for unknown seller
  - `/stores/:seller/:slug` 404s for unknown product
  - `/api/resolve` returns JSON for a known product
  - short-URL `/s/:shortcode` 302-redirects to canonical path
  - listing endpoint only shows `status: active` stores

## Verification steps

1. `HONE_GATEWAY_ENABLED=true node src/index.js` starts both API (3000) and gateway (4343)
2. `curl http://localhost:4343/stores` returns HTML with alice's store
3. `curl http://localhost:4343/stores/alice/widget` returns HTML product page
4. `curl http://localhost:4343/api/resolve/stores/alice/widget` returns JSON
5. Kill the gateway, API still works (gateway is additive)

## Effort

Small — this is ~400 lines of gateway code + tests. All business logic already exists in stateStore.

## Deferred to later phases (do not build in v2.10.2)

- `/fs/:cid` blob streaming — arrives in v2.11 alongside HONE-FS
- `/service/:slug` reverse proxy — arrives in v2.13 alongside SERVICE_DEPLOY
- `.hone` TLD / browser extension resolution — out of scope, v2.15+ polish
- Signed response proofs (gateway proves data came from chain) — v2.12+ trust-minimization hardening
- Cart / checkout UI with payment flow — v2.10.3 UX phase

## Gateway trust model

- **Local mode** (default): gateway runs in the same process or on the same machine as a full HONE node, reads stateStore directly. Zero-trust wrt chain data — the reader IS the chain.
- **Remote mode** (opt-in via `HONE_GATEWAY_REMOTE_RPC_URL`): gateway is a lightweight HTTP server that queries a remote HONE node via a new `/api/rpc` JSON-RPC endpoint. Trust the node it queries. Good for phones, browser extensions, kiosks.

Both modes share the same routes and rendering code. `resolver.js` abstracts the data source.

---

# Plan revisions — v2.10.2 through v2.15 additions (2026-04-10 session)

This section captures design decisions made during planning conversation after the original plan was drafted. User approved all.

## Addition A — Oracles folded into v2.13 (stateless compute)

New ledger entry type **`ORACLE_REPORT`** + verifier-median consensus:
```
{
  type: 'ORACLE_REPORT',
  from: 'oracle-operator-1',
  epoch,
  oracle_data: {
    feed_id: 'price.hone.usd',   // or any namespace (weather.austin, sports.nba.lakers)
    value: '0.0425',                 // string-encoded to preserve precision
    source_sig: '...',               // optional: proof from an external source
    source_url_hash: '...',          // hash of queried endpoint, NOT the URL
    confidence: 0.95,                // optional reporter confidence
  }
}
```

**Consensus**: verifier panel aggregates N reports with the same `feed_id` per epoch. Accepted value is median. Outliers >2σ get slashed. Consumers query `stateStore.getOracle(feedId)` for the current consensus value with timestamp.

**New stateStore:**
- `oracles` Map: `feed_id → { value, confidence, reporters: [...], epoch_updated, report_count }`
- New entry type: `ORACLE_SUBSCRIBE` — consumer locks escrow to subscribe; oracle operators earn pro-rata for contributing.
- New `oracleReports` Map (per-epoch buffer, cleared after finality): `"feed_id|epoch" → [{ from, value, sig }]`

**Unlocks**: commerce pricing in USD (bonding curve becomes real), DeFi primitives, prediction markets, time-sensitive auctions, real-world-aware contracts, sensor data feeds (see v2.15).

## Addition B — VRF verifiable randomness folded into v2.12 (fee market)

New ledger entry type **`VRF_COMMIT`** — one per clock node per epoch:
```
{
  type: 'VRF_COMMIT',
  from: 'clock-node-a',
  epoch,
  vrf_data: {
    beacon_seed: sha256(prev_beacon + epoch),
    vrf_proof: ecvrf_proof(node_secret_key, beacon_seed),
    vrf_output: 'hex...'
  }
}
```

**Consensus**: XOR all valid VRF outputs for an epoch → epoch beacon. `stateStore.getRandomBeacon(epoch)` returns it. Manipulation-resistant: a single honest clock node is enough to randomize the result.

**Implementation**: use `@noble/curves` ECVRF or equivalent BLS/VRF library. ~200 LoC total.

**Unlocks**: provably fair games (dice, cards, loot), unmanipulable NFT mints, lottery systems, verifier panel random sampling (latent improvement over current deterministic selection), governance sortition.

## Addition C — Bandwidth accounting folded into v2.11 (HONE-FS)

Extend `BLOB_STORE_COMMIT` with separate storage + bandwidth pricing:
```
blob_data: {
  cid,
  hosts,
  duration_epochs,
  payment_hone,            // storage rate (per-epoch)
  bandwidth_rate_hone,     // per-GB served (new)
  ...
}
```

New entry type **`BLOB_SERVE_PROOF`** — hosts report bytes served each epoch:
```
{
  type: 'BLOB_SERVE_PROOF',
  from: 'storage-host-a',
  epoch,
  blob_data: {
    cid,
    bytes_served,
    request_count,
    access_log_merkle_root,   // pre-committed per request
    challenge_sample_proofs   // responses to verifier spot-checks
  }
}
```

**Honesty check**: verifier panel periodically fetches specific byte ranges from hosts and compares against Merkle proofs. Fraudulent reports → slashing.

**Unlocks**: honest CDN economics, per-request payment (not just idle capacity), natural geographic distribution of hot content, gateway economics (gateways relay bytes, they earn too).

## Addition D — NODE_REGISTER capability extension folded into v2.10.2

Change `account_data.node_type` (string) → `account_data.node_types` (array):
```
{
  type: 'NODE_REGISTER',
  from: 'shindevlin',
  account_data: {
    node_types: ['miner', 'verifier', 'storage_host', 'gateway_op'],
    p2p_address,
    storage_capacity_gb: 2000,         // optional, only if storage_host
    service_capacity: { cpu, ram_gb },  // optional, only if service_host
    lora_region: 'US915',              // optional, only if sensor_bridge
    permissioned
  }
}
```

Backward compatibility: if an entry has legacy `node_type` string, dispatcher treats it as `[node_type]`. Old entries still replay correctly.

**Reputation becomes per-capability axis** (already supported via v2.10.0 target_type):
- `reputation["miner|shindevlin"]`
- `reputation["storage_host|shindevlin"]`
- `reputation["gateway_op|shindevlin"]`
- `reputation["store|shindevlin"]`

This is the most important cross-cutting change. All subsequent phases just add new values to the enum.

## Addition E — New phase v2.15: HONE-nano + Helium miner repurpose

Positioned after v2.14 in the build order (requires HONE-FS + oracles + sensor support). Can start marketing/docs early though.

### Goal
Turn the dormant Helium miner fleet into a HONE node fleet. Five income streams on hardware the owner already bought: clock, storage_host, gateway_op, verifier, sensor_bridge.

### The LoRa sensor mesh primitive (the killer feature)

New ledger entry types:

- **`SENSOR_REGISTER`** — small stake (~0.1 HONE) to register a sensor ID and claim its feed_id. Sensor owner generates a key pair, registers the public key on chain. Feed namespace: `sensor.<category>.<owner>.<slug>`.

- **`SENSOR_READING`** — LoRa gateway relays a signed reading from a sensor. Treated as a scoped `ORACLE_REPORT` — goes through verifier consensus if multiple gateways witness the same sensor, gets recorded as authoritative reading.

- **`SENSOR_SUBSCRIBE`** — consumer (farmer, city, researcher) subscribes to a sensor's feed, locks escrow, receives pro-rata releases as readings flow in.

### HONE-nano specification
- Debian-based raw disk image (`hone-nano.img`) built from Raspberry Pi OS Lite
- Node.js + HONE codebase preinstalled
- `lora_pkt_fwd` + `sx1302_hal` drivers for Semtech concentrators
- systemd service auto-starts node
- First-boot web wizard on port 80 (10-minute window): create/import account, pick node name, confirm capabilities
- Default capabilities: `clock`, `storage_host`, `gateway_op`, `sensor_bridge`
- Per-model overlays in `configs/nano-overlays/` (RAK, SenseCAP, Nebra, Bobcat, Linxdot, etc.)

### New code files
- `src/sensors/loraBridge.js` — receives LoRa packets from concentrator, validates signatures, emits SENSOR_READING entries
- `src/sensors/loraProtocol.js` — packet format: `[sensor_id(8)][epoch(4)][value(N)][sig(64)]`
- `src/sensors/sensorRegistry.js` — tracks registered sensors and their feed mappings
- `src/nano/firstBootWizard.js` — web UI for first-boot setup
- `src/nano/systemMonitor.js` — thermal throttle awareness, eMMC wear monitoring, power-draw aware scheduling
- `bin/hone-nano` — thin launcher for the nano build
- `configs/nano-overlays/*.json` — per-model hardware maps (SPI paths, GPIO reset pins, LoRa region defaults)
- `scripts/flash-nano.sh` — SD card + eMMC flashing helper

### Documentation
- `docs/helium-repurpose.md` — per-model flashing guide
- `docs/sensor-bridge-setup.md` — deploying LoRa sensors, registering feeds, subscribing
- `docs/nano-hardware-list.md` — tested hardware, per-model notes, known issues

### Per-model support at launch
Target the ~95% that are RPi-based and SD-card-accessible first. Explicit support list:
- ⭐ Trivial: RAK Hotspot v1/v2, RAK 2287, SenseCAP M1, Syncrob.it, original Helium Hotspot, MNTD Finestra
- ⭐⭐ Medium (eMMC via usbboot): Nebra CM4 Indoor/Outdoor, Linxdot, Bobcat 300/500, Panther X/X1/X2
- ⛔ Skip initially: Kerlink iFemtoCell, MultiTech Conduit (custom SoCs, non-RPi)

### Open questions (flagged for user decision before coding starts)
1. **Own-brand hardware vs BYO?** Recommendation: BYO first.
2. **Sensor ownership model** — owner-sponsored vs gateway-sponsored? Recommendation: both, tracked separately.
3. **Data privacy** — encrypted feeds? Recommendation: v2.15.1 follow-up, ship public feeds first.
4. **LoRa regional regulation** — US915/EU868/AS923? Recommendation: region field in NODE_REGISTER + SENSOR_REGISTER, auto-detected from IP geolocation with user confirmation.

## Deferred / explicit out-of-scope (documented here so the plan file remembers)

These are interesting but NOT part of v2.10.1-v2.15. Revisit in v2.16+ or beyond.

- **Training compute** (GPU beyond inference, fine-tuning, RLHF) — natural v2.14.1 extension once SERVICE_DEPLOY is shipped. Different verification scheme (checkpoint hashes + partial re-runs).
- **Scheduled execution** ("cron on HONE") — SCHEDULED_EXEC entry for time-delayed execution. Small addition, ~200 LoC. v2.12.1.
- **Proof-of-location / edge compute** — RTT triangulation from known anchors, latency-priced hosting. Complex (GPS spoofing resistance is hard). v2.16+.
- **Residential IP proxy** — real market but DDoS/fraud vectors too high. Skip.
- **Human labor / RLHF / data labeling** — subjective tasks corrode objective consensus. Better as third-party marketplace ON HONE, not IN HONE.
- **IoT beyond LoRa** (5G, NB-IoT, Sigfox) — different radio protocols; LoRa first because of Helium hardware.
- **KYC / identity attestation** — legal/regulatory risk. Third parties can build on top.
- **Native tokens on other chains** (stable bridging, wHONE on multiple chains) — partial existing work, not a v2.10-2.15 priority.
- **Governance system** — out of scope entirely; leave as-is, address in a dedicated governance phase later.
- **Mobile native apps** — web + PWA covers it until demand justifies native.

## Updated build order (final for this session)

```
v2.10.0  ✅ Commerce primitive (stores, products, orders, reputation, bonding curve)  [shipped]
v2.10.1  🔨 Commerce HTTP routes                                                      [in progress]
v2.10.2  🔨 Gateway + NODE_REGISTER capability list generalization
v2.11.x  🔨 HONE-FS blob store + bandwidth accounting (BLOB_SERVE_PROOF)
v2.12.x  🔨 Block cap + fee market + VRF beacon (VRF_COMMIT)
v2.13.x  🔨 Stateless compute (SERVICE_DEPLOY) + oracles (ORACLE_REPORT)
v2.14.x  🔨 Stateful compute (STATE_SNAPSHOT + STATE_TRANSFER)
v2.15.x  🔨 HONE-nano + LoRa sensor mesh + Helium miner repurpose
```

Each phase is independently committable with version bump (patch bumps for sub-phases: v2.11.0, v2.11.1, v2.11.2 as needed).

## Execution proceeds from v2.10.1 immediately after plan commit.

# BTCPC Whitepaper Implementation Notes

Created: 2026-04-14

## Current State

- Repository had no tracked modifications at the start of this pass.
- Untracked runtime/local files were present and intentionally ignored:
  - `.btcpc-image-hash`
  - `btcpc/`
  - `data/gnss/`
  - `data/known-peers.json`
  - `data/seen-messages.json`
  - `scripts/swap-eth-to-matic.js`

## Whitepaper Audit Findings To Fix

- `src/services/storageChallenge.js` accepts any response of the expected length. It must verify the hash of the challenged byte range.
- P2P signing defaults are not whitepaper-strict. Protected/significant messages should require signatures by default, with only explicit local legacy/test opt-out.
- Some protocol paths still fall back to 300000 ms epochs. Whitepaper and user clarification require 30000 ms epochs and clock-node consensus.
- Username validation differs from the whitepaper and should reserve `wallet`.
- TOTP exists in service/routes, but protocol/SDK enforcement needs a concrete integration model.
- External finality anchoring now uses real block/state data where available and persists anchor history to disk. Remaining work is wallet-funded/sponsor-funded submission flow and bridge proof verification wiring.
- TON derivation is seeded from the same BIP-39 mnemonic as BTCPC role keys and the other chain wallets, but it still exposes a raw public-key identifier instead of a contract-derived TON account address.
- Service/oracle/bridge modules have useful primitives, but several headers/comments still say chain dispatch lands later.

## Concrete Next Files

- `src/chain/anchorSubmission.js` — keep expanding the batch proof format and wire submitters into an actual external-chain broadcaster.
- `src/services/bridgeRegistry.js` — snapshot persistence is now in place; remaining work is moving bridge accounting into the ledger/state pipeline without changing reserve invariants.
- `src/wallet/keyManager.js` — if real TON account addresses are required, add contract-aware derivation with the existing TON packages and keep the linkable pubkey path as a fallback.
- `src/routes/bridgeRoutes.js` — once bridge persistence exists, expose those records through the REST layer instead of the current registry snapshot.

## Work Log

- 2026-04-14: Created this handoff log and implementation plan.
- 2026-04-14: Aligned account username validation with the protocol rule across auth/bot/account creation paths, including `wallet` reservation and no leading/trailing hyphen support.
- 2026-04-14: TOTP service/routes were inspected but not expanded; SDK/client still lacks first-class TOTP helpers or protocol-enforced signing flows.
- 2026-04-14: Updated `requireTOTP` so human wallets backed by local secretStore records can opt into TOTP and have it enforced on protected routes, instead of enforcing only Mongo-backed users.
- 2026-04-14: User clarified the intended TOTP UX: standard authenticator app generates a 6-digit code. TOTP seeds must stay private/off-chain; chain should store only public policy/commitment if needed.
- 2026-04-14: Added non-secret `TOTP_POLICY` ledger/state entries for on-chain TOTP policy/commitment. Raw TOTP seed and backup codes remain off-chain.
- 2026-04-14: P2P auth now defaults to strict signatures, with BTCPC_REQUIRE_SIGNATURES=false reserved for local legacy/test opt-out; protocol BLOCK_PROPOSAL fallback now uses 30s epochs instead of 300s.
- 2026-04-14: Removed the P2P `BTCPC_EPOCH_DURATION_MS` override for block proposal validation; protocol fallback logic now uses the fixed 30-second consensus epoch duration.
- 2026-04-14: Aligned TON derivation with chainLink by exposing the raw ed25519 public key as a linkable identifier derived from the shared BIP-39 mnemonic; no bridge/finality persistence changes were made in this pass.
- 2026-04-14: Updated `src/services/storageChallenge.js` to compute an expected range hash from BTCPC-FS and reject responses whose bytes do not hash to that value. Added `tests/storageChallenge.test.js`.
- 2026-04-14: User clarified the remembered emission doubling cadence: periods start at 1 week, then 2 weeks, 4 weeks, 8 weeks, and continue doubling.
- 2026-04-14: Interpreted the weekly doubling schedule as constant reward-per-epoch inside each period, so each period's total allotment doubles; the 42M cap truncates the final period and consensus only needs an epoch-to-reward lookup.
- 2026-04-14: Updated `src/services/emissionSchedule.js` and `tests/epochTiming.test.js` to use weekly doubling periods with a constant reward-per-epoch until the 42M supply cap. Updated explorer tokenomics copy for weekly doublings.
- 2026-04-14: Updated `src/chain/anchorSubmission.js` to use real block/state-derived anchor payloads, include batch Merkle roots, and persist anchor history to `data/anchor-history.json` unless `BTCPC_ANCHOR_HISTORY_PATH` is set. Added persistence tests.
- 2026-04-14: Added JSON snapshot persistence hooks to `src/services/bridgeRegistry.js` with an overridable `BTCPC_BRIDGE_SNAPSHOT_PATH`. Added reload tests. Ledger/state pipeline integration still remains for a later pass.
- 2026-04-14: Tightened sensor/gateway POST routes so account identity comes from authenticated user middleware instead of spoofable `body.account` fallback.
- 2026-04-14: Removed implicit sensor auto-registration from reading submission; unknown sensors now fail closed with 404 and must be registered first.
- 2026-04-14: Bumped package metadata to 2.16.3 for this whitepaper implementation branch.
- 2026-04-14: Added Jest test discovery config so `npm test` ignores unrelated/untracked `btcpc/` TypeScript app tests and only runs this repo's `tests/**/*.test.js` suite.
- 2026-04-14: Aligned `src/mining/rewardDistribution.js` with the whitepaper's six-pool emission split: 55% miner, 10% verifier, 5% clock, 12% storage, 8% service, 10% IoT, with unclaimed pools recycling to `btcpc_recycle`.
- 2026-04-14: Added verifier scaling policy and explicit P2P verifier opt-in via `BTCPC_VERIFIER_ENABLED=true` or `BTCPC_NODE_ROLE=verifier`; early networks verify lightly, larger networks increase panel size/coverage.
- 2026-04-14: Added explorer `/api/visualizer` plus Telegram Mini App chain visualizer showing epochs, work roles, sensors/gateways, anchors, bridge state, rewards, and state root.
- 2026-04-14: Added public explorer `/visualizer` page backed by `/api/visualizer/stream` SSE. The visualizer now reports role instances, not physical PC count, so one machine can appear as clock + miner + verifier + storage/service/gateway roles.
- 2026-04-14: User clarified the desired visualizer is "The Global Nervous System": a high-end WebGL geospatial experience with a 3D cinematic globe and 2D command-map mode. Keep the current card visualizer as the audit/readout layer, but put the cinematic globe/map above it as the public-facing hook.
- 2026-04-14: Visualizer privacy model: do not show exact node locations. Map nodes to metro-area buckets (Tokyo, Singapore, London, Frankfurt, New York, Los Angeles, Sao Paulo, Mexico City, Lagos, Dubai, Mumbai, Sydney, etc.) and place each node deterministically in a 15-50km metro-sector ring so even users who are physically next door do not render as neighbors. This is more intentionally wide than simple coordinate jitter.
- 2026-04-14: Visualizer creative direction: dark-matter Earth / midnight cyber-industrial theme, no country borders or names, glowing hex hubs, magenta inference arcs, cyan storage trails, exponential pulse decay, additive glow/bloom-style materials, camera inertia, auto-rotating globe, flat command-map toggle, and a translucent matrix-style live ledger synced to pulses.
- 2026-04-14: Current visualizer implementation uses Three.js from jsDelivr in `src/explorer/views/visualizer.js` for the cinematic top layer, derives fuzzed metro positions in-browser from role/node identifiers, consumes `/api/visualizer/stream` SSE, and leaves `/api/visualizer` JSON as fallback. If CDN dependency is unacceptable later, vendor Three.js or replace with a local WebGL renderer.

## Verification

- PASS: `npx jest tests/storageChallenge.test.js --runInBand`
- PASS: `npx jest tests/epochTiming.test.js tests/storageChallenge.test.js --runInBand`
- PASS: `npx jest --runInBand tests/accountManagerUsername.test.js tests/authController.test.js tests/authControllerSecretStore.test.js tests/botApiRedesign.test.js tests/totpSecretStore.test.js tests/projectRoutesSecretStore.test.js tests/p2pMessageAuth.test.js tests/p2pSecurityIntegration.test.js tests/epochTiming.test.js tests/storageChallenge.test.js tests/keyManager.test.js tests/chainLink.test.js tests/anchorSubmission.test.js tests/bridgeRegistry.test.js`
- PASS: `npx jest tests/sensorRoutes.test.js --runInBand`
- PASS: `npx jest tests/p2pMessageAuth.test.js tests/p2pSecurityIntegration.test.js tests/epochTiming.test.js tests/sensorRoutes.test.js --runInBand`
- PASS: `npx jest tests/totpSecretStore.test.js tests/authMiddlewareSecretStore.test.js tests/walletController.test.js tests/projectRoutesSecretStore.test.js --runInBand`
- PASS: `npx jest tests/rewardDistribution.test.js tests/anchorSubmission.test.js tests/bridgeRegistry.test.js tests/keyManager.test.js tests/inferenceVerifier.test.js tests/totpSecretStore.test.js tests/epochTiming.test.js --runInBand`
- PASS: `npx jest tests/nodeRegistry.test.js tests/rewardDistribution.test.js tests/anchorSubmission.test.js tests/bridgeRegistry.test.js tests/keyManager.test.js tests/inferenceVerifier.test.js tests/totpSecretStore.test.js tests/epochTiming.test.js --runInBand`
- PASS: `node --check src/explorer/server.js`
- PASS: `node --check src/explorer/views/visualizer.js`
- PASS: `git diff --check`
- LIMITED: `npx jest --passWithNoTests --runInBand` originally picked up unrelated untracked `btcpc/` TypeScript tests before Jest discovery was scoped.
- LIMITED: `timeout 180s npx jest --passWithNoTests --runInBand` after scoping timed out without output, so this branch is currently verified by focused touched-area suites rather than the full suite.

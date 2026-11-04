# HONE Whitepaper Implementation Notes

## 2026-11-04: white paper v2.0 rewrite, docs vs code discrepancies

What I checked: `docs/HONE_WHITEPAPER.md` v1.1 (July 2026), the old `HONE_WHITEPAPER.pdf`, and
`docs/index.html`. I compared them against `v2.1-live-identity` at 4ed83bf2 (`rust/hone-node`,
`crates/hone-types`), the uncommitted working tree on grouchly (branch `net-nat`),
`CHAIN_AGENT_PROGRESS.md`, `docs/spec-sightings-coverage-rewards.md`, hone-sound
`android/sound-sensing` (SIGHTINGS.md, APP_AGENT_PROGRESS.md), hone-firmware `hone`
(SIGHTINGS.md), hone-decide (JURY.md), and the live honemesh.net (Caddyfile, endpoints, DNS).

Rule applied: where docs and code disagreed, the code won. The v2.0 paper describes the
code and labels each mechanism live, gated or planned.

| # | Topic | Docs said | Code / reality | Resolution |
|---|---|---|---|---|
| 1 | Inference | v1.1: inference runs in-process (candle) so verifiers can re-run it | f767c022: the node never loads a model in-process; `hone-node inference-serve` sidecar or any Ollama/OpenAI-compatible server at `INFERENCE_URL`; a light node never runs a model | Paper describes the sidecar. In-process determinism claim removed |
| 2 | Model registry | `ModelManifestPublish` / `ModelUsageAttest` described as live chain primitives | No such ledger entries | Marked planned |
| 3 | Bitcoin oracle | `BtcHeightReport` entries with validity rules | Only comments in `emission.rs`; no entry variant | Marked planned; compile-time projection is used |
| 4 | Wiiv | Native ledger entries from genesis | `rust/wiiv` holds types and the state machine only, "no chain wiring yet" | Marked planned; reserved accounts noted |
| 5 | Typed addresses | `hh1…` bech32 addresses, rejected at parse time | No `hh` HRP in the node; bech32 only for the Cosmos-style external wallet | Marked planned |
| 6 | Peer discovery | `bootstrap1/2.honemesh.net`, `_hone._udp.honemesh.net` TXT | None of these names resolve (checked against 1.1.1.1). Public node `/dns4/honemesh.net/tcp/6942` is in node env configs and the app (hone-sound 31121b1e, 7b0237e5). Code default `/dnsaddr/bootstrap.honemesh.net` + VPS is uncommitted on `net-nat` | Public-first: live via configs. DNS seed: planned. Old names dropped |
| 7 | NAT traversal | Not covered | libp2p `relay`, `dcutr`, `autonat`, `upnp` features are uncommitted on `net-nat` | Marked planned / in development |
| 8 | `MAX_PEERS = 50` | Stated | Not found in node code | Removed |
| 9 | Links | Explorer `scan.honemesh.net`, Telegram `@honebot` | scan.honemesh.net has no DNS record; the Telegram bot is unverified | Removed from the paper's links. The website still lists the Telegram bot; not changed |
| 10 | Role keys | Brief listed five (owner, active, posting, memo, hide) | `wallet.rs` derives six, including `seek` | Paper keeps all six |
| 11 | Key display | "Seed phrase displayed once and never stored" (contradicts the keystore section) | Recoverable keystore is mandatory | Contradiction removed |
| 12 | Sensor rewards | Baseline per reading plus purchase premium | Volume `sensor_score(reading_count)` still pays. `USEFULNESS_SCORING_EPOCH = u64::MAX` (352,700 withdrawn, 878c8aa8). `USEFULNESS_PAYOUT_EPOCH = u64::MAX`. Deferred claim: corroboration or `CLAIM_WINDOW_EPOCHS = 2880`. `MAX_CLAIM_AGE_S` = 7 days. NOVEL 100, VERIFY 1 | Paper describes the gated usefulness model and states that volume scoring is live until activation |
| 13 | Decide stake | Spec: 100 HONE per voting device | `NodeRoleStake` accepts any amount > 0; `decide_nodes()` counts any non-zero stake | Paper: "intended, not enforced yet". Enforcement is a code TODO |
| 14 | decide_jury.rs comment | `decide_nodes` doc says "device seats excluded" | The body includes device seats (by design from 352,440) | Code wins. Stale comment flagged for the chain agent |
| 15 | Identity NFT | `IDENTITY_NFT_ACTIVATION_EPOCH = 333_333` commented "PLACEHOLDER / PROPOSED" | The epoch has passed, and the 352,000 migration created 1,162 `NameNft`s | Treated as live. Stale comment flagged |
| 16 | Hardware tiers | Implied attested tiers | Tier from `HONE_HW_TIER` env, weights 1/4/8/16/32, no attestation | Paper says self-declared; attestation planned |
| 17 | Reserved names | Website: "420 reserved names at genesis" | 1,162 names migrated at 352,000 (CHAIN_AGENT_PROGRESS) | Website updated |
| 18 | Website security cards | "Four-key hierarchy", "72-hour recovery" time-lock | Six role keys; no 72-hour recovery found. Recovery is identity-NFT guardian recovery to a pinned destination | Website cards rewritten |
| 19 | PDF | The PDF should match the .md | The old PDF was "BTCPC v0.3, March 2026" (pandoc/xelatex), never rebuilt from v1.1 | Rebuilt from v2.0 with pandoc + xelatex |
| 20 | Licence | Brief: keep UFL references as in the current doc | v1.1 had no UFL reference. Repo `LICENSE` says "UFL Version 1.1"; the project notes say Hone is adopting UFL 3.4 (Operational Scope: Decentralized) | Added a version-less pointer to `LICENSE`. `LICENSE` itself was not touched and needs the owner's update |
| 21 | Device status | Brief: signed status every 15 min (planned) | `DeviceStatus.kt` exists but is uncommitted in hone-sound; `POST /api/devices/status` returns 404 | Planned, with the 15-minute cadence stated |
| 22 | Healer | Deterministic cursor | 9be59e7b (E−21) is merged on the branch but not deployed | "merged, not deployed" |
| 23 | Speech removal | Spec: verifiable removal, public test missing under 1% | The app drops any frame where Perch sees a human class; the public test is not built | Live (drop-frame), with verifiable test planned before mic data earns |
| 24 | Find-My on phones | SIGHTINGS.md: "no hide key on phones" | hone-sound 36ea6958 restored hide-key import (owner's choice); importing hide clears a hardware grant. `FIND_GRANT_EPOCH = u64::MAX` | Paper states both: grants gated; hide import is the interim option |
| 25 | Genesis accounts | "17 founding accounts" | `genesis-required-accounts.txt` has 32 lines | Count removed |
| 26 | Pools | April notes: fixed 55/10/5/12/8/10 split | Rust emission: dynamic pools, `LAYER_A_POOL_COUNT = 9` | Paper lists the pools without fixed percentages |
| 27 | Site source | Brief: is `docs/index.html` the source? | No. `docs/index.html` is a docsify shell. honemesh.net serves `/opt/hone-site/website` on ferryvps, a sparse clone of `shindevlin/hone` **main** `website/` | Live files edited in place (see the website section below) |
| 28 | `/app/` | Expected 200 | 404 (no index file); `HEAD /api/birds/health` returns 501 (GET returns 200) | Added `/var/www/hone-app/index.html`. Caddy unchanged |

Not reviewed directly: the Hone project docs (`claude/hone-*.md`). This session had no Projects tool. I used the repo copy of the sightings spec and the code. Where the decide-jury, birds, identity, hardware-tiers, passage, ledger-split, flipper and ops docs add detail beyond the code, someone with project access should cross-check the paper against them. The Passage vertical is not covered in the paper.

### Website (honemesh.net) changes, 2026-11-04

- Backup: `/root/hone-site-backup-20261104-<time>/` on ferryvps (copy of `website/index.html` and `/var/www/hone-app` listing).
- `website/index.html`:
  - hero buttons for the white paper and the app;
  - a new "What runs today" status section;
  - phone role and sensor text updated (usefulness rewards gated);
  - reward box sensor line;
  - security cards (six role keys, name NFTs with recovery, private by design);
  - 420 changed to 1,162 names;
  - APK links pointed to `/app/hone-latest.apk`;
  - white paper link in the footer.
- `website/HONE_WHITEPAPER.pdf`: new file.
- `/var/www/hone-app/index.html`: new install page with the sha256.
- The clone at `/opt/hone-site` now has local changes. A `git pull` there will refuse to overwrite them until the same edits land on `main` `website/`.

---

## History: April 2026 (JS prototype era)

Created: 2026-04-14

## Current State

- Repository had no tracked modifications at the start of this pass.
- Untracked runtime/local files were present and intentionally ignored:
  - `.hone-image-hash`
  - `hone/`
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
- TON derivation is seeded from the same BIP-39 mnemonic as HONE role keys and the other chain wallets, but it still exposes a raw public-key identifier instead of a contract-derived TON account address.
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
- 2026-04-14: P2P auth now defaults to strict signatures, with HONE_REQUIRE_SIGNATURES=false reserved for local legacy/test opt-out; protocol BLOCK_PROPOSAL fallback now uses 30s epochs instead of 300s.
- 2026-04-14: Removed the P2P `HONE_EPOCH_DURATION_MS` override for block proposal validation; protocol fallback logic now uses the fixed 30-second consensus epoch duration.
- 2026-04-14: Aligned TON derivation with chainLink by exposing the raw ed25519 public key as a linkable identifier derived from the shared BIP-39 mnemonic; no bridge/finality persistence changes were made in this pass.
- 2026-04-14: Updated `src/services/storageChallenge.js` to compute an expected range hash from HONE-FS and reject responses whose bytes do not hash to that value. Added `tests/storageChallenge.test.js`.
- 2026-04-14: User clarified the remembered emission doubling cadence: periods start at 1 week, then 2 weeks, 4 weeks, 8 weeks, and continue doubling.
- 2026-04-14: Interpreted the weekly doubling schedule as constant reward-per-epoch inside each period, so each period's total allotment doubles; the 42M cap truncates the final period and consensus only needs an epoch-to-reward lookup.
- 2026-04-14: Updated `src/services/emissionSchedule.js` and `tests/epochTiming.test.js` to use weekly doubling periods with a constant reward-per-epoch until the 42M supply cap. Updated explorer tokenomics copy for weekly doublings.
- 2026-04-14: Updated `src/chain/anchorSubmission.js` to use real block/state-derived anchor payloads, include batch Merkle roots, and persist anchor history to `data/anchor-history.json` unless `HONE_ANCHOR_HISTORY_PATH` is set. Added persistence tests.
- 2026-04-14: Added JSON snapshot persistence hooks to `src/services/bridgeRegistry.js` with an overridable `HONE_BRIDGE_SNAPSHOT_PATH`. Added reload tests. Ledger/state pipeline integration still remains for a later pass.
- 2026-04-14: Tightened sensor/gateway POST routes so account identity comes from authenticated user middleware instead of spoofable `body.account` fallback.
- 2026-04-14: Removed implicit sensor auto-registration from reading submission; unknown sensors now fail closed with 404 and must be registered first.
- 2026-04-14: Bumped package metadata to 2.16.3 for this whitepaper implementation branch.
- 2026-04-14: Added Jest test discovery config so `npm test` ignores unrelated/untracked `hone/` TypeScript app tests and only runs this repo's `tests/**/*.test.js` suite.
- 2026-04-14: Aligned `src/mining/rewardDistribution.js` with the whitepaper's six-pool emission split: 55% miner, 10% verifier, 5% clock, 12% storage, 8% service, 10% IoT, with unclaimed pools recycling to `hone_recycle`.
- 2026-04-14: Added verifier scaling policy and explicit P2P verifier opt-in via `HONE_VERIFIER_ENABLED=true` or `HONE_NODE_ROLE=verifier`; early networks verify lightly, larger networks increase panel size/coverage.
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
- LIMITED: `npx jest --passWithNoTests --runInBand` originally picked up unrelated untracked `hone/` TypeScript tests before Jest discovery was scoped.
- LIMITED: `timeout 180s npx jest --passWithNoTests --runInBand` after scoping timed out without output, so this branch is currently verified by focused touched-area suites rather than the full suite.

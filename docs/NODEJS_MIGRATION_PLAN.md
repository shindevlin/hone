# Node.js → Rust Migration Plan

38 route files surveyed. Status legend: ✅ Done in Rust · 🔄 Partial · ⬜ Not started · 🗄️ Archive

---

## Phase 0 — Archive now (not worth porting)

| File | Reason |
|------|--------|
| `recoveryRoutes.js` | Explicitly disabled (all routes return 503); security concern with unsigned owner challenges — do not port |
| `dreamRoutes.js` | Thin router delegating to a controller; unclear if feature is still alive — audit before any port |
| `amberPillRoutes.js` | Read-only geo-pioneer NFT lookup; no state machine, no writes — can be a static JSON query on any future API |

---

## Phase 1 — Chain core (block these on launch)

Already in Rust node but incomplete, or gaps that directly affect chain usability.

| # | File | What's missing in Rust | Complexity |
|---|------|------------------------|------------|
| 1 | `explorerRoutes.js` | `/status`, `/supply`, `/blocks`, `/block/:n`, `/accounts`, `/account/:name/history`, `/network`, `/reputation` — the full public explorer surface | Medium |
| 2 | `walletRoutes.js` | Nested accounts (`/create-nested`, `/unnest`), alias transfer, cross-chain identity link, `/transactions` history | Medium |
| 3 | `stakingRoutes.js` | `/stake/policy`, `/stake/sponsor`, `/stake/withdraw`, `/stake/network`, `/stake/requirements` | Small |
| 4 | `nodeRoutes.js` | `/node/register`, `/node/update`, `/node/deregister`, `/epoch/commit`, `/node/list`, `/miners/:name`, `/network/capabilities` | Medium |
| 5 | `faucetRoutes.js` | Testnet faucet `/claim` and `/status` (IP + account rate limited) | Small |

---

## Phase 2 — Inference & job marketplace

Rust already has `/api/task/*` covering the core inference flow. Node.js `inferenceMarketRoutes.js` is the predecessor — compare endpoints, port any gaps, then retire it.

| # | File | What's missing in Rust | Complexity |
|---|------|------------------------|------------|
| 6 | `inferenceMarketRoutes.js` | Tool-call relay (`/:id/tool-calls`, `/:id/tool-result`), batch job posting, `/jobs/claimed` listing — audit vs existing `/api/task/*` | Large |
| 7 | `finetuneRoutes.js` | Full LoRA fine-tuning marketplace — post job, miner claims, progress, complete, refund | Medium |
| 8 | `computerUseRoutes.js` | Browser automation marketplace — post job, screenshot, action, complete | Medium |
| 9 | `streamingRoutes.js` | SSE streaming inference with per-token billing (`/stream`, `/jobs/:id/stream`) | Small |
| 10 | `sessionMarketRoutes.js` | Persistent inference sessions with context (`/sessions`, `/:id`, `/:id/summary`) | Small |
| 11 | `sessionRoutes.js` | Third-party delegated session tokens with spending limits | Small |
| 12 | `appealRoutes.js` | Slash record appeal + verifier panel resolution | Small |

---

## Phase 3 — Storage & files

| # | File | What's missing in Rust | Complexity |
|---|------|------------------------|------------|
| 13 | `blobRoutes.js` | Content-addressed blob upload/download (HONE-FS CID system) | Medium |
| 14 | `storageRoutes.js` | Encrypted file storage, shard management, grant/revoke access | Large |
| 15 | `blobServeProofRoutes.js` | Bandwidth-proof submission and flush for storage host payouts | Small |
| 16 | `phoneStorageRoutes.js` | Phone storage host registration, blob assignments, heartbeat | Small |

---

## Phase 4 — Commerce, bridge, purchase

| # | File | What's missing in Rust | Complexity |
|---|------|------------------------|------------|
| 17 | `commerceRoutes.js` | Full storefront/product/order/escrow/dispute/reputation system | Large |
| 18 | `bridgeRoutes.js` | HONE ↔ wHONE wrap/unwrap, LP provisioning, fee distribution | Medium |
| 19 | `purchaseRoutes.js` | Stablecoin → HONE purchase flow (ETH/SOL/TON multi-chain quotes + fulfillment) | Medium |
| 20 | `auctionRoutes.js` | Name auction: open, bid, settle, cancel, delegate | Medium |
| 21 | `peerCommerceRoutes.js` | Read-only peer commerce catalog (stores, products, health) | Small |

---

## Phase 5 — Platform services

| # | File | What's missing in Rust | Complexity |
|---|------|------------------------|------------|
| 22 | `serviceRoutes.js` | Stateless service hosting — deploy, retire, heartbeat, sessions, snapshots, restore | Large |
| 23 | `modelRoutes.js` | Model registry — upload, fetch from HuggingFace, manifest, file serving | Large |
| 24 | `sensorDataRoutes.js` | Sensor data marketplace — rate card, coverage, availability, quote, query | Medium |
| 25 | `oracleRoutes.js` | Oracle feed management — verifier reports, median finalization, deviation flagging | Medium |
| 26 | `projectRoutes.js` | GitHub project registration, verification, funding, revenue splits | Medium |
| 27 | `delegationRoutes.js` | Delegation to miners — delegate, undelegate, withdraw, list | Small |
| 28 | `memoryRoutes.js` | Per-account memory CRUD with project management and graph view | Small |
| 29 | `toolRegistryRoutes.js` | MCP tool registration for agentic inference jobs | Small |
| 30 | `toolRoutes.js` | Native tool execution and MCP server management | Small |

---

## Phase 6 — Auth & device

| # | File | What's missing in Rust | Complexity |
|---|------|------------------------|------------|
| 31 | `publicRoutes.js` | `/signup` (AccountCreate UI flow), `/device-heartbeat`, `/leaderboard`, `/model-demand`, `/android-version`, `/my-devices`, `/machine-status` — JWT login not needed (Rust uses key-based auth) | Medium |
| 32 | `userRoutes.js` | Telegram link, 2FA enable/verify, MCP server self-management | Small |
| 33 | `totpRoutes.js` | TOTP setup, enable, verify, disable, backup codes | Small |
| 34 | `phoneMiningRoutes.js` | Phone mining synthetic work unit claim and submit | Small |

---

## Phase 7 — Telegram bot API surface

`botRoutes.js` is large (~50 endpoints) but lives in the separate bot repos (`~/repos/honebot/`, `~/repos/honewalletbot/`). Those bots call the Rust node's `/api/*` endpoints — not the Node.js stack. As Phase 1–6 routes land in Rust, the bots point at those directly. No explicit port needed; just keep bot routes wired to Rust endpoints as they go live.

---

## Summary

| Phase | Route files | Priority | Blocker for launch? |
|-------|-------------|----------|---------------------|
| 0 — Archive | 3 | — | No |
| 1 — Chain core | 5 | Critical | Yes |
| 2 — Inference/jobs | 7 | High | Soft (core inference already in Rust) |
| 3 — Storage | 4 | High | Yes (storage host rewards) |
| 4 — Commerce/bridge | 5 | Medium | No |
| 5 — Platform services | 9 | Medium | No |
| 6 — Auth/device | 4 | Medium | No |
| 7 — Bot API | 1 | Low (auto-follows) | No |

**Archive condition**: once all phases are complete and the Node.js server has been offline for one release cycle, move `src/` and `bin/` to `_archived/nodejs-prototype/`.

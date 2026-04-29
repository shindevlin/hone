# BTCPC Docs Index

> Open `docs/` as an Obsidian vault. This file is the entry point. Every link below is wikilink-friendly — Obsidian will resolve them.

## Protocol Businesses

- [[NATIVE_PROTOCOLS]] — Freeport, Verasens, and LinkGit: the three native protocol businesses deployed at genesis, how they generate fees, and their standalone licensing value

## Architecture

- [[BTCPC_WHITEPAPER]] — full whitepaper, Appendix M (Decentralized Commerce Layer), inscribed on Dream #0
- [[TOKENOMICS]] — canonical economic model: 42M supply, No Burn All Recycle, fees, Area Pioneers
- [[governance]] — sovereignty model, founder-irrelevance design
- [[founders]] — what BTCPC is for and why
- [[bots]] — Telegram bot architecture (`@btcpcbot`, `@btcpcwalletbot`)
- [[reports/ETHEREUM_SOLANA_STYLE_REVIEW]] — code, security, and documentation review with Ethereum/Solana-style doc recommendations
- [[reports/IMPLEMENTATION_SPEC_FOR_SECURITY_REVIEW]] — implementation handoff with exact files, insertion points, and code shapes for security fixes
- [[reports/CHAIN_REMEDIATION_OPTIONS]] — recovery paths for the negative-balance replay issue: no-genesis fix, hard reset, or consensus fork
- [[reports/RELAUNCH_RESET_PLAN]] — proposed reset-first relaunch plan, wallet migration, and doc rewrite checklist
- [[reports/RELAUNCH_COPY_DRAFTS]] — launch copy notes for the whitepaper, README, and website, with balance policy and reset rationale
- [[reports/RUST_CHAIN_PRIMARY_MIGRATION_PLAN]] — Rust-first migration plan for all chain-critical logic, invariants, and replay
- [[reports/RUST_PORT_MATRIX_DETERMINISTIC_CONTRACTS]] — module-by-module Rust portability matrix and deterministic smart-contract runtime blueprint
- [[reports/RUST_CUTOVER_AUDIT_GATES]] — phase-by-phase pass/fail audit gates for Rust consensus cutover

## Protocols

- [[FREEPORT_PROTOCOL]] — Freeport native marketplace protocol: entry types, key roles, escrow, digital product delivery
- [[FREEPORT_PROTOCOL_WHITEPAPER]] — full Freeport whitepaper (Shin Devlin, v3.1)
- [[VERASENS_PROTOCOL]] — Verasens IoT sensor/device protocol: entry types, device key registration, reserved accounts
- [[LINKGIT_PROTOCOL]] — LinkGit decentralized git protocol: repos on btcpc-fs, on-chain refs, private repos via hide key encryption

## Commerce

- [[BTCPC_WHITEPAPER#Appendix M: Decentralized Commerce Layer]] — architecture, escrow mechanics, ledger entry types
- `website/store.html` — static store frontend; configurable `API_BASE` via `?node=` or `localStorage`
- `website/vendor.html` — vendor control panel (same static file, accessed with signing key)
- `btcpc-market` — Rust service (port 7042), optional vendor operations sidecar

## Implementation plan

- [[PLAN_v2.10.1_to_v2.14]] — multi-phase plan covering commerce → BTCPC-FS → block-cap → service hosting → stateful compute
- [[CLAUDE_HANDOFF_2026-04-08]] — session handoff for cleanup/test pass

## What's shipped (v2.13)

- **v2.10** — commerce: stores, products, orders, reputation, bonding curve
- **v2.10.1** — commerce HTTP routes
- **v2.10.2** — gateway skeleton for discoverability
- **v2.11** — BTCPC-FS content-addressed blob storage with bandwidth + 2-tier active/cold
- **v2.12** — scaling discipline: block size cap (1 MB), fee market, VRF beacon
- **v2.13** — stateless compute hosting
  - alpha: serviceRegistry primitive
  - beta: serviceHostRunner (host-side execution)
  - gamma: serviceRoutes HTTP layer
  - delta: oracleFeeds verifier-median consensus
- **v2.13.1** — cross-process ledger queue (entries actually land in blocks)
- **v2.13.2** — multi-role Docker entrypoint (api+miner+clock+storage)

## What's pending

- **D.5-delta** — TOTP + project routes secretStore migration
- **Phase E** — delete chain-state Mongoose models
- **Phase F** — MongoDB optional
- **v2.14** — stateful compute with snapshot replication
- **v2.15** — BTCPC-nano + LoRa sensor mesh + Helium miner repurpose
- **v2.16** — four-tier finality + lock-and-recycle bridge

## Quick links

- Run miner: `systemctl --user status btcpc-miner` or `node bin/btcpc-mine --miner shindevlin`
- Mongo: `mongodb://root:example@localhost:27017/btcpc?authSource=admin`
- Explorer: `localhost:4242`
- P2P: `localhost:6942`
- Multi-role supervisor: `node bin/btcpc-all` (with `BTCPC_ROLES=all`)

## Specs

- Supply: 42,000,000 BTCPC (1 BTCPC = 100M dreams)
- Genesis reward: 243.06 BTCPC/epoch (5-min epochs)
- 420 reserved premium names

## Hard rules

- **No burn, ever.** All fees flow to `btcpc_recycle`. See [[TOKENOMICS]] §5.
- **Storage is never slashed.** Pay for delivery, not for absence. See `~/.claude/projects/-home-ubuntclaw-repos-btcpc/memory/feedback_storage_no_slash.md`
- **Token standard:** every BTCPC token (native + user-issued) is 42M supply, 10 decimals
- **No fixed BTCPC promises.** Always % of stream / fraction of pool / share of rewards

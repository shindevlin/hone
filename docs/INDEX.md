# BTCPC Docs Index

> Open `docs/` as an Obsidian vault. This file is the entry point. Every link below is wikilink-friendly — Obsidian will resolve them.

## Architecture

- [[BTCPC_WHITEPAPER]] — full whitepaper, 12 appendices, inscribed on Dream #0
- [[TOKENOMICS]] — canonical economic model: 42M supply, No Burn All Recycle, fees, Area Pioneers
- [[governance]] — sovereignty model, founder-irrelevance design
- [[founders]] — what BTCPC is for and why
- [[bots]] — Telegram bot architecture (`@btcpcbot`, `@btcpcwalletbot`)

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

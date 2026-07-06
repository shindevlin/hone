# HONE — Sovereign Chain Node

> **Naming (BTCPC → HONE rebrand, locked):** brand/network = **HONE**, token/ticker = **HONE**,
> base unit = **hunit** (1 HONE = 10^10 hunits), chain_id = **hone** (mainnet) / **hone-testnet**,
> crates = `hone-*`, CLI binary = `hone`, env vars = `HONE_*`, domain = **honemesh.net**.
> Treat any remaining "BTCPC/btcpc/dream" as a not-yet-migrated tail, not the current name.

> **THE ACTIVE IMPLEMENTATION IS RUST. DO NOT REFERENCE OR MODIFY NODE.JS CODE.**
>
> `src/`, `bin/`, `package.json`, `node_modules/` — these are the **deprecated** Node.js prototype.
> They are kept only for historical reference. All active development happens in `rust/hone-node/`.

## Active Stack

- **Rust** — single binary, `rust/hone-node/src/main.rs`
- **libp2p** — gossipsub P2P networking (port 6942)
- **RocksDB** — chain state (not MongoDB)
- **Axum** — HTTP API (port 4242)
- **Embedded candle GGUF inference** — `inference_engine::chat` (external OpenAI/Ollama-compatible server optional via `INFERENCE_URL`); see docs/INFERENCE_INTEGRATION_FOR_CONSUMERS.md
- Git author: Shin Devlin <shindevlin@proton.me> (NO AI attribution ever)

## Key Files (Rust)

- `rust/hone-node/src/main.rs` — node entry point, epoch seal handler, reward wiring
- `rust/hone-node/src/chain.rs` — state machine, apply_entry, pending pool
- `rust/hone-node/src/clock.rs` — clock consensus, epoch sealing, quorum
- `rust/hone-node/src/tx.rs` — validate_and_apply, is_system_entry
- `rust/hone-node/src/api.rs` — Axum HTTP API routes
- `rust/hone-node/src/hardware.rs` — GPU serial / machine-id anti-sybil
- `rust/hone-node/crates/hone-types/src/entry.rs` — LedgerEntry enum (canonical)
- `rust/hone-node/crates/hone-types/src/emission.rs` — supply/reward schedule
- `rust/hone-node/genesis.json` — mainnet genesis block
- `website/` — landing page

## Run

```bash
systemctl --user status hone-node      # check Rust node
systemctl --user restart hone-node     # restart after binary update
curl http://localhost:4242/api/node/info
```

## Key Specs

- Supply: 42,000,000 HONE (1 HONE = 10,000,000,000 hunits; canonical constant `HUNITS_PER_HONE` in `crates/hone-types/src/lib.rs`)
- Epoch duration: 30 seconds
- Genesis timestamp: 1783191600000 (2026-07-04 noon Los Angeles, PDT). Do not change.
- Chain ID: hone-testnet (testnet), hone (mainnet)
- Explorer: port 4242, P2P: port 6942

## Architecture

Entries flow: **gossip received → Chain::pending pool → epoch seals → drain_pending_sorted → validate_and_apply in sha256 order → RocksDB**

System entries (EpochSeal, ClockReward, MineReward, etc.) apply immediately on epoch seal — they do NOT go through the pending pool.

The epoch IS the block. `signing_clocks` in SealedEpoch is who gets ClockReward.

## Hardline: No Local Submission Without Peers

**A node with zero peers MUST NOT accept or apply any user-submitted entry.**

If a node applies entries locally while disconnected from the network, it silently forks — its chain state diverges from the canonical chain with no way to reconcile. The user sees transactions as confirmed; the network has never seen them. This is a chain integrity failure.

The rule:
- `apply_and_broadcast` in `api.rs` rejects all submissions when `peer_count == 0`
- Do NOT remove or soften this check
- Do NOT add a "local mode" or "offline mode" that bypasses it
- Do NOT show submitted entries as confirmed until they have been sealed in an epoch by the network

This applies to ALL entry types: stakes, transfers, registrations, sensor data, everything. There are no exceptions.

## Monorepo Structure

```
btcpc/                   (repo dir name predates the HONE rebrand)
  rust/
    hone-node/       ← chain node (canonical)
    hone-node/crates/hone-types/
    hone-p2p/        ← libp2p DHT sidecar
    hone-market/     ← commerce HTTP service
    hone-gnss-capture/ ← GNSS RTCM3 capture
  ludicrous/             ← Warp fork (HONE terminal)
    plugins/ludicrous/   ← Claude Code plugin
  clients/
    btcpc-desktop/       ← Electron/Tauri desktop app (dir not yet renamed)
    btcpc-android/       ← Capacitor Android client (dir not yet renamed)
    btcpc-flipper/       ← Flipper Zero firmware (dir not yet renamed)
  bots/
    btcpcbot/            ← Telegram chain bot (@btcpcbot; rename pending)
    btcpcwalletbot/      ← Telegram wallet bot (rename pending)
  services/
    btcpc-relay/         ← Cloudflare Workers relay (dir not yet renamed)
  marketing/             ← Open-source marketing
  website/               ← honemesh.net landing page
  verasens/              ← Sensor verification protocol
  linkgit/               ← Ed25519 Git-native identity layer
  freeport/              ← NAT-traversal P2P port router
```

## Telegram Bots

See [docs/bots.md](docs/bots.md) for full bot documentation.
- Bots are thin HTTP clients — no direct DB access, all via `/api/bot/*`
- Live in `bots/btcpcbot/` and `bots/btcpcwalletbot/` (in this repo)
- Tokens in `.env` files only — NEVER in git, NEVER in chat (.gitignore guards in place)

## DEPRECATED: Node.js (do not use)

`src/`, `bin/`, `package.json` — legacy Node.js prototype, v3.x era.
Do not run, do not modify, do not reference in new code.
Kept for migration reference only. Will be archived.

<!-- code-review-graph MCP tools -->
## MCP Tools: code-review-graph

**IMPORTANT: This project has a knowledge graph. ALWAYS use the
code-review-graph MCP tools BEFORE using Grep/Glob/Read to explore
the codebase.** The graph is faster, cheaper (fewer tokens), and gives
you structural context (callers, dependents, test coverage) that file
scanning cannot.

### When to use graph tools FIRST

- **Exploring code**: `semantic_search_nodes` or `query_graph` instead of Grep
- **Understanding impact**: `get_impact_radius` instead of manually tracing imports
- **Code review**: `detect_changes` + `get_review_context` instead of reading entire files
- **Finding relationships**: `query_graph` with callers_of/callees_of/imports_of/tests_for
- **Architecture questions**: `get_architecture_overview` + `list_communities`

Fall back to Grep/Glob/Read **only** when the graph doesn't cover what you need.

### Key Tools

| Tool | Use when |
|------|----------|
| `detect_changes` | Reviewing code changes — gives risk-scored analysis |
| `get_review_context` | Need source snippets for review — token-efficient |
| `get_impact_radius` | Understanding blast radius of a change |
| `get_affected_flows` | Finding which execution paths are impacted |
| `query_graph` | Tracing callers, callees, imports, tests, dependencies |
| `semantic_search_nodes` | Finding functions/classes by name or keyword |
| `get_architecture_overview` | Understanding high-level codebase structure |
| `refactor_tool` | Planning renames, finding dead code |

### Workflow

1. The graph auto-updates on file changes (via hooks).
2. Use `detect_changes` for code review.
3. Use `get_affected_flows` to understand impact.
4. Use `query_graph` pattern="tests_for" to check coverage.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

Rules:
- ALWAYS read graphify-out/GRAPH_REPORT.md before reading any source files, running grep/glob searches, or answering codebase questions. The graph is your primary map of the codebase.
- IF graphify-out/wiki/index.md EXISTS, navigate it instead of reading raw files
- For cross-module "how does X relate to Y" questions, prefer `graphify query "<question>"`, `graphify path "<A>" "<B>"`, or `graphify explain "<concept>"` over grep — these traverse the graph's EXTRACTED + INFERRED edges instead of scanning files
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).

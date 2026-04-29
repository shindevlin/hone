# Freeport Protocol

Sovereign blockchain for compute, storage, and commerce — no gatekeepers.

## Stack
- Node.js, MongoDB, WebSocket P2P
- Ollama for mining inference — model-agnostic. Miners run any Ollama model (qwen, llama, mistral, gemma, deepseek, etc.). Work value scales with verified parameter count from Ollama's /api/show, not the model name.
- Git author: Shin Devlin <shindevlin@proton.me> (NO Claude attribution ever)

## Key Files
- `bin/btcpc-mine` — mining daemon CLI
- `bin/btcpc-cli` — wallet/transaction CLI
- `src/models/` — 10 Mongoose models (Epoch, Wallet, Transaction, etc.)
- `src/controllers/` — auth, wallet, staking, node, dream, delegation, recovery
- `src/network/` — P2P WebSocket, peer discovery, chain sync
- `docs/FREEPORT_PROTOCOL_WHITEPAPER.md` — full whitepaper (inscribed on Dream #0)
- `website/` — landing page

## Run
```bash
systemctl --user status btcpc-miner  # check miner
node bin/btcpc-mine --miner shindevlin  # manual mine
```

## Key Specs
- Supply: 42,000,000 BTCPC (1 BTCPC = 100M dreams)
- Current package version: 3.0.87
- Genesis timestamp: 1776236400000 (2026-04-15T07:00:00.000Z). Do not change.
- Epochs: 30 seconds
- Genesis reward: 243.06 BTCPC/epoch during bootstrap; current reward logic lives in `src/chain/blockProposal.js`
- MongoDB: optional (post-Phase F). Default: disabled. Set BTCPC_MONGO_MODE=enabled and MONGODB_URI=mongodb://root:example@localhost:27017/btcpc?authSource=admin to re-enable for legacy migration.
- Explorer: port 4242, P2P: port 6942

## Version Rule
- When bumping BTCPC version, update `package.json` and `package-lock.json` in the same change.
- Verify with: `node -e "const p=require('./package.json'); const l=require('./package-lock.json'); if (p.version !== l.version || p.version !== l.packages[''].version) process.exit(1)"`
- Do not commit a version bump if those three values differ.

## Telegram Bots
See [docs/bots.md](docs/bots.md) for full bot documentation.
- Bots are thin HTTP clients — no direct DB access, all via `/api/bot/*`
- Live in standalone repos: `~/repos/btcpcbot/`, `~/repos/btcpcwalletbot/`
- Tokens in `.env` files only — NEVER in git, NEVER in chat
- Kill zombies before starting (see bots.md for commands)

## Current State
- Genesis miner (shindevlin) running on GPU node
- 420 reserved premium names
- Whitepaper v0.3 complete with 12 appendices

## Session Notes
- Detailed handoff notes for the 2026-04-08 cleanup/test pass live in `docs/CLAUDE_HANDOFF_2026-04-08.md`
- Use that handoff before touching auth, tests, tracker docs, or CI added in that pass

## Autopilot
- A Ralph-loop driver lives at `~/.btcpc-autopilot/` (operator config, NOT in this repo)
- Cron-driven; reads `docs/SELF_HEAL_PRD.md`, picks the next unticked item, dispatches one agent in a worktree, cherry-picks, tests, commits, pushes, ticks the box
- Setup walkthrough: `~/.btcpc-autopilot/README.md` (claude setup-token → env file → manual test → crontab line)
- `docs/SELF_HEAL_PRD.md` IS the autopilot's spec — every fix lives there as a checkbox
- Hard rule for ALL fail paths: `feedback_self_heal_no_asks.md` in memory — never ask the user to do something technical, always auto-repair

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

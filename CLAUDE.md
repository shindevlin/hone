# btcpc

Bitcoin Proof of Compute — sovereign chain for AI inference.

## Stack
- Node.js, MongoDB, WebSocket P2P
- Ollama for mining inference — model-agnostic. Miners run any Ollama model (qwen, llama, mistral, gemma, deepseek, etc.). Work value scales with verified parameter count from Ollama's /api/show, not the model name.
- Git author: Shin Devlin <shin@btcpc.network> (NO Claude attribution ever)

## Key Files
- `bin/btcpc-mine` — mining daemon CLI
- `bin/btcpc-cli` — wallet/transaction CLI
- `src/models/` — 10 Mongoose models (Epoch, Wallet, Transaction, etc.)
- `src/controllers/` — auth, wallet, staking, node, dream, delegation, recovery
- `src/network/` — P2P WebSocket, peer discovery, chain sync
- `docs/BTCPC_WHITEPAPER.md` — full whitepaper (inscribed on Dream #0)
- `website/` — landing page

## Run
```bash
systemctl --user status btcpc-miner  # check miner
node bin/btcpc-mine --miner shindevlin  # manual mine
```

## Key Specs
- Supply: 42,000,000 BTCPC (1 BTCPC = 100M dreams)
- Genesis reward: 243.06 BTCPC/epoch (5 min epochs)
- MongoDB: optional (post-Phase F). Default: disabled. Set BTCPC_MONGO_MODE=enabled and MONGODB_URI=mongodb://root:example@localhost:27017/btcpc?authSource=admin to re-enable for legacy migration.
- Explorer: port 4242, P2P: port 6942

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

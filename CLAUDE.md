# btcpc

Bitcoin Proof of Compute — sovereign chain for AI inference. Genesis miner running on Beastly.

## Stack
- Node.js, MongoDB, WebSocket P2P
- Ollama (qwen3.5:27b) for mining inference
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
- MongoDB: mongodb://root:example@localhost:27017/btcpc?authSource=admin
- Explorer: port 4242, P2P: port 6942

## Telegram Bots
See [docs/bots.md](docs/bots.md) for full bot documentation.
- Bots are thin HTTP clients — no direct DB access, all via `/api/bot/*`
- Live in standalone repos: `~/repos/btcpcbot/`, `~/repos/btcpcwalletbot/`
- Tokens in `.env` files only — NEVER in git, NEVER in chat
- Kill zombies before starting (see bots.md for commands)

## Current State
- Genesis miner (shindevlin) mining on Beastly
- 420 reserved premium names
- Whitepaper v0.3 complete with 12 appendices

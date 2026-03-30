# Bitcoin Proof of Compute (BTCPC)

A sovereign blockchain where mining means useful AI inference. Miners earn BTCPC by providing real GPU compute to the network. Every token is backed by verified work.

**Total supply: 42,000,000 BTCPC** — the answer to life, the universe, and everything.

## What BTCPC Does

- Miners run AI models (Ollama) and earn BTCPC for each epoch of verified inference
- Users submit encrypted inference requests — prompts are private, even from the node operator
- Cross-chain rewards: mining on BTCPC automatically generates claimable wBTCPC on linked chains (Hive, Base)
- Silicon Identity Keys (SIK) bind encryption to physical GPU hardware — no other chain has this

## Node Types

| Type | GPU Required | What It Does | Guide |
|------|-------------|-------------|-------|
| **Inference Miner** | Yes | Runs AI models, earns BTCPC | [docs/install-miner.md](docs/install-miner.md) |
| **User Node** | No | Wallet, submit requests, trade | [docs/install-user.md](docs/install-user.md) |
| **Validator** | No | Validates blocks, relays, no mining | [docs/install-validator.md](docs/install-validator.md) |

## Quick Start (Miner)

```bash
git clone https://github.com/estejosh/btcpc.git
cd btcpc
npm install
cp .env.example .env   # edit with your config
```

**Requirements:** Node.js 20+, MongoDB, Ollama with a supported model

**Start mining:**
```bash
node bin/btcpc-mine
```

**Start API + Explorer:**
```bash
npm start                          # API on :3000
node src/explorer/server.js        # Explorer on :4242
```

## Environment Variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `MONGODB_URI` | Yes | `mongodb://localhost:27017/btcpc` | MongoDB connection string |
| `OLLAMA_URL` | Yes | `http://100.122.145.60:11434` | Ollama inference endpoint |
| `BTCPC_MODEL` | No | `qwen3.5:27b` | Model for mining inference |
| `BTCPC_WORK_PER_EPOCH` | No | `3` | Inference tasks per epoch |
| `P2P_PORT` | No | `6942` | WebSocket P2P port |
| `BTCPC_SEED_PEERS` | No | — | Comma-separated peer addresses (`ws://host:6942`) |
| `PORT` | No | `3000` | API server port |
| `JWT_SECRET` | Yes | — | JWT signing secret |
| `ALERTBOT_URL` | No | — | Alertbot endpoint for monitoring |
| `ALERTBOT_API_KEY` | No | — | Alertbot API key |

## Supported Models

Any Ollama model works. Weight factors scale rewards with model size:

| Model Size | Weight | Examples |
|-----------|--------|---------|
| 1B-7B | 1.0x | phi3, gemma2:2b |
| 7B-13B | 2.0x | qwen3.5:9b, llama3.1:8b, deepseek-r1:8b |
| 13B-30B | 4.0x | qwen3.5:27b, mixtral |
| 30B-70B | 8.0x | qwen3-coder:30b |
| 70B+ | 16.0x | llama3.1:70b |

## Architecture

```
┌─────────────┐  ┌──────────────┐  ┌───────────────┐
│ Mining       │  │ P2P Network  │  │ Inference API │
│ Daemon       │  │ (WebSocket)  │  │ (Encrypted)   │
│              │  │              │  │               │
│ Ollama ←───→│←→│ Peers ←────→│←→│ Users         │
│ Work proofs  │  │ Block gossip │  │ SIK-bound     │
│ Epoch commit │  │ Peer discovery│ │ sessions      │
└─────────────┘  └──────────────┘  └───────────────┘
        │                │                  │
        └────────────────┴──────────────────┘
                         │
              ┌──────────┴──────────┐
              │  MongoDB            │
              │  Accounts, Wallets, │
              │  Epochs, Proofs,    │
              │  Stakes, Claims     │
              └─────────────────────┘
```

## CLI

```bash
node bin/btcpc-cli status    # Network status
node bin/btcpc-cli balance   # Account balance
node bin/btcpc-cli mining    # Mining stats
```

## Telegram Bot

[@btcpcbot](https://t.me/btcpcbot) — check balances, mining stats, network info

## Documentation

- [Whitepaper](docs/BTCPC_WHITEPAPER.md) ([PDF](docs/BTCPC_WHITEPAPER.pdf))
- [API Reference](docs/api.md)
- [Getting Started](docs/getting-started.md)
- [Architecture](docs/L2_ARCHITECTURE.md)

## Key Innovations

- **Proof of Compute** — mining produces useful AI inference, not wasted hashes
- **Proof of Silicon** — GPU fingerprinting binds encryption to physical hardware
- **Zero-Plaintext Inference** — prompts tokenized client-side, remapped, never exist as text on the node
- **Anti-Centralization** — newcomer bonus + concentration penalty prevents mining monopolies
- **Cross-Chain Mining** — earn on BTCPC + claimable wBTCPC on every linked chain

## License

MIT

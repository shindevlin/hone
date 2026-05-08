# Bitcoin Proof of Compute (BTCPC)

**Mine by doing useful work.** GPU, Raspberry Pi, phone, spare drive, or AI agent. Every token earned by a machine doing something real.

> **v0.4.0 — Testnet.** The chain is live and earning real rewards on testnet. Protocol hardening is in progress before mainnet (v1.0.0). See the [Roadmap](docs/ROADMAP.md) for what's being built and what's open.

---

## What it is

BTCPC is a blockchain where mining means running AI inference, covering BLE tracker networks, hosting encrypted storage, and keeping network time — not grinding hashes that prove nothing. Every token in existence was earned by a machine doing real work.

Three native protocol businesses create token demand from genesis:

- **Freeport** — peer-to-peer sovereign marketplace. No platform ban, no 15% cut, no payment processor reversal. Escrow-protected orders, instant digital delivery encrypted to the buyer's key, dispute resolution on-chain.
- **Verasens** — decentralized sensor data network. Sensor operators register devices, submit readings, and earn from query fees. Data consumers pay BTCPC per query. Every reading is traceable to a device key and epoch.
- **LinkGit** — Git-compatible decentralized code hosting on BTCPC-FS. Repos are content-addressed and encrypted at rest. Private repos use the hide/seek key pair — no storage node can read the content.

These are native ledger entry types, not smart contracts deployed after the fact. They are as native to BTCPC as a token transfer is native to Bitcoin.

---

## Stack

- **Rust** — `rust/btcpc-node` — single binary: libp2p networking, sled state, clock consensus, inference mining, Axum HTTP API (port 4242)
- **Rust** — `rust/btcpc-android` — standalone Android micronode (libp2p + sled + Candle inference, fully self-contained)
- **Ollama** — AI inference backend, model-agnostic (qwen, llama, mistral, gemma, deepseek, etc.)
- **libp2p** — gossipsub P2P mesh, port 6942

> `src/` (Node.js) is kept for historical reference only. The canonical chain is `rust/btcpc-node`.

---

## Protocol Status

| Feature | Status |
|---------|--------|
| Chain + consensus | Live — testnet |
| Inference mining | Live — testnet |
| Freeport commerce | Live — testnet |
| Verasens sensors | Live — testnet |
| LinkGit | Live — testnet |
| BLE tracker network | Live — testnet |
| Clock node rewards | Live — testnet |
| Storage rewards | Live — testnet |
| Chain entropy (liveness decay) | Designed — not yet enabled |
| Smart contracts | Not planned — BTCPC is native-protocol-only |
| Formal consensus finality | In progress (Phase 2) |
| Storage challenge proofs | In progress (Phase 4) |
| Inference job linkage | In progress (Phase 3) |
| Bridge multisig | In progress (Phase 6) |
| External security audit | Planned (Phase 8) |

See [docs/ROADMAP.md](docs/ROADMAP.md) for the full hardening plan.

---

## Install

One command installs both `btcpc-node` (the chain node) and `btcpc` (the CLI):

```bash
curl -fsSL https://btcpc.net/install.sh | sudo bash
```

Requires Ubuntu/Debian x86-64. Installs binaries to `/usr/local/bin`, creates the `btcpc` system user, and enables daily auto-update.

**After install — create your account:**

```bash
btcpc account-create yourname
btcpc login
```

**Publish a git repo:**

```bash
cd my-project
btcpc repo init my-project
git push -u origin main
```

**Run a node:**

```bash
sudo nano /etc/systemd/system/btcpc-node.service
# Set: BTCPC_ACCOUNT, BTCPC_NODE_ID, BTCPC_GENESIS_TIMESTAMP=1777633200000, BTCPC_CHAIN_ID=btcpc-1
systemctl enable --now btcpc-node
```

Check the node:

```bash
systemctl status btcpc-node
curl http://localhost:4242/api/node/info
```

**Build from source:**

```bash
git clone https://github.com/shindevlin/btcpc
cd btcpc/rust/btcpc-cli && cargo build --release
cd ../btcpc-node && cargo build --release
sudo cp rust/btcpc-cli/target/release/btcpc /usr/local/bin/
sudo cp rust/btcpc-node/target/release/btcpc-node /usr/local/bin/
```

### Docker

```bash
docker run -d \
  -e BTCPC_ACCOUNT=yourname \
  -p 4242:4242 -p 6942:6942 \
  btcpc/node
```

### Running over a VPN (recommended for privacy)

```ini
# In /etc/systemd/system/btcpc-node.service
Environment="BTCPC_P2P_ANNOUNCE_ADDR=/ip4/10.x.x.x/tcp/6942"
```

---

## Earn BTCPC

Every device earns by doing useful work. Reward pools are demand-driven — no fixed percentages. Pools with more real activity earn more of the epoch reward.

| Role | What it does | Hardware |
|------|-------------|----------|
| **Clock** | Keeps epoch timing alive | Anything — phone, Pi, laptop |
| **Miner** | AI inference via Ollama | Any computer (GPU earns more) |
| **Sensor** | BLE trackers, environmental data, GNSS | Android phone, Raspberry Pi, LoRa gateway |
| **Storage** | Hosts BTCPC-FS blobs | Spare SSD or HDD |
| **Service** | Runs containers for the network | VPS or server |

Multiple roles stack. A Raspberry Pi running BLE scanning + clock earns from two pools simultaneously.

### GPU Mining

```bash
curl -fsSL https://ollama.ai/install.sh | sh
ollama pull qwen2.5:7b
BTCPC_ACCOUNT=yourname btcpc-node
```

### Raspberry Pi (BLE Tracker + Clock)

```bash
BTCPC_ACCOUNT=yourname \
BTCPC_BLE_TRACKER=true \
btcpc-node
```

The Pi passively scans for AirTags, Android Find My, Tile, and Samsung SmartTags. Owners of tracked devices pay a subscription fee to receive encrypted sighting data — a share goes to observer nodes.

### Phone

Download the [Android APK](https://github.com/shindevlin/btcpc/releases/latest/download/BTCPC-android-release.apk) or open [btcpc.net/app](https://btcpc.net/app). Enable sensors and clock from the app UI. First epoch reward within 30 seconds.

### Telegram Wallet

```
/create yourname
```

Message [@btcpcbot](https://t.me/btcpcbot). No install required.

---

## Use BTCPC inference in your project

Drop-in replace for OpenAI:

```javascript
const OpenAI = require('openai');
const client = new OpenAI({
  baseURL: 'https://btcpc.net/v1',
  apiKey: process.env.BTCPC_API_KEY
});

const response = await client.chat.completions.create({
  model: 'auto',
  messages: [{ role: 'user', content: 'Explain quantum computing' }]
});
```

Or curl:

```bash
curl -X POST https://btcpc.net/v1/chat/completions \
  -H "Authorization: Bearer btcpc_your_key" \
  -H "Content-Type: application/json" \
  -d '{"model": "auto", "messages": [{"role": "user", "content": "Hello"}]}'
```

Get an API key: create an account via [@btcpcbot](https://t.me/btcpcbot), then `POST /api/faucet/claim`.

---

## Key Numbers

| Parameter | Value |
|-----------|-------|
| Total supply | 42,000,000 BTCPC |
| Smallest unit | 1 dream (1 BTCPC = 10,000,000,000 dreams) |
| Genesis timestamp | 1777633200000 ms (2026-05-01, noon Ireland, UTC+1) |
| Era 0 epoch duration | 30 seconds |
| Era 0 block reward | 2 BTCPC per epoch |
| Emission model | Epoch duration doubles every 4,200,000 epochs |
| Supply exhaustion | ~124 years from genesis (~2150) |
| Chain ID (mainnet) | btcpc-1 |
| Chain ID (testnet) | btcpc-satoshi |
| HTTP API port | 4242 |
| P2P port | 6942 |

---

## Key Concepts

**Role-based keys.** Every BTCPC account derives 6 keys from one BIP-39 mnemonic: `owner` (cold storage, recovery), `active` (financial operations), `posting` (chain entries), `memo` (encrypt messages), `hide` (receive encrypted goods), `seek` (deliver encrypted goods). The hide/seek pair powers private commerce and private code hosting — payloads travel on-chain, only the key holder can decrypt.

**No burn, all recycle.** Fees, slashes, and unclaimed pool rewards flow to `__recycle_fund__`. Nothing is burned. The 42M supply ceiling is a hard maximum, not a target that erodes through burning.

**Epoch-duration doubling.** Instead of halving the per-epoch reward (Bitcoin model), BTCPC doubles the epoch duration every 4,200,000 epochs. Daily emission halves either way — but the per-epoch reward stays constant within an era, making reward modeling predictable for miners and tools.

**Chain entropy.** Accounts that prove liveness (any key signature, or cross-chain activity on a published wallet address) maintain full balance. Accounts with no activity signal for an extended period gradually return a portion of their balance to the recycle fund — re-entering the reward pool, never destroyed. Full documentation will be published on the website and whitepaper before this feature is enabled.

**Useful-work proof.** Mining rewards trace to real inference jobs, not hash puzzles. A `Mine` entry references an `InferenceJobPost`. Reward flows only after an independent verifier approves the work. Fake or unlinked mine entries earn a reduced grace-period rate during network bootstrap, then zero.

---

## Roadmap

BTCPC is at **v0.4.0 (testnet)**. The path to v1.0.0 (hardened mainnet) covers 8 phases:

| Phase | Target Version | Focus |
|-------|---------------|-------|
| 0 | v0.4.1 | Freeze and fix two confirmed broken reward bugs |
| 1 | v0.5.0-alpha | State machine safety: chain ID, unbonding, governance |
| 2 | v0.5.0 | Consensus finality, fork choice, double-sign slashing |
| 3 | v0.6.0-alpha | Useful-work proof linkage, inference job requirement |
| 4 | v0.6.0 | Storage challenge proofs, sensor anti-fraud |
| 5 | v0.7.0-alpha | Dynamic fee market (no burn) |
| 6 | v0.7.0 | Bridge multisig, Solana/Bitcoin signature support |
| 7 | v0.8.0 | Patricia Merkle state proofs, light client |
| 8 | v1.0.0 | Adversarial testnet, external audit, mainnet launch |

Full detail including 37 problem items, design decisions, and open questions: [docs/ROADMAP.md](docs/ROADMAP.md)

---

## For agents and developers

See [AGENTS.md](AGENTS.md) for the agent onboarding guide, repo map, and contribution rules.

Agent onboarding prompt:

```
You are working on BTCPC (Bitcoin Proof of Compute). Read AGENTS.md,
CLAUDE.md, and README.md first. Use the code-review-graph MCP tools
before Grep or file scanning. The canonical chain is rust/btcpc-node
(port 4242). Do not reference or modify the Node.js src/ directory.
Do not change genesis constants, token economics, or security-sensitive
flows unless explicitly asked.
```

---

## Links

- **Website:** [btcpc.net](https://btcpc.net)
- **Telegram:** [@btcpcbot](https://t.me/btcpcbot)
- **Explorer:** [btcpc.net/dashboard](https://btcpc.net/dashboard)
- **Whitepaper:** [docs/BTCPC_WHITEPAPER.md](docs/BTCPC_WHITEPAPER.md)
- **Roadmap:** [docs/ROADMAP.md](docs/ROADMAP.md)
- **Reddit:** [r/btcpc](https://reddit.com/r/btcpc)
- **Substack:** [btcpc.substack.com](https://btcpc.substack.com)

---

## License

MIT

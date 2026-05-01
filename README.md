# Bitcoin Proof of Compute (BTCPC)

**Mine by doing useful work.** GPU, Raspberry Pi, phone, spare drive, or AI agent. Every token earned by a machine doing something real.

> **Chain is live.** Install now and start earning.
> See [Install a node](#install-a-node) below.

---

## What it is

BTCPC is a blockchain where mining means running AI inference, covering BLE tracker networks, hosting encrypted storage, and keeping network time — not grinding hashes that prove nothing.

Three native markets create real token demand from day one:

- **Verasens** — sensor data marketplace. Telecoms, logistics companies, and researchers pay BTCPC to query verified on-chain sensor data. Sensor nodes earn from query fees.
- **Freeport** — peer-to-peer commerce without a platform between buyer and seller. No 15% cut, no account bans. Storage nodes and service nodes earn from marketplace activity.
- **LinkGit** — decentralized, Git-compatible code hosting on BTCPC-FS. Repos are content-addressed, permanent, and encrypted at rest. Storage nodes earn per object stored.

This is where mining rewards come from. Not just inflation — real economic activity.

---

## Stack

- **Rust** — `rust/btcpc-node` — single binary: libp2p networking, RocksDB state, clock consensus, miner, WASM contracts, Axum HTTP API
- **Rust** — `rust/btcpc-contract-sdk` — BSP-20 / BSP-721 smart contract SDK
- **Rust** — `rust/btcpc-ble-tracker` — passive BLE tracker oracle for Pi and desktop nodes
- Ollama — AI inference backend, model-agnostic (qwen, llama, mistral, gemma, deepseek, etc.)

> `src/` (Node.js) is retained for reference only. The canonical chain is `rust/btcpc-node`.

---

## Install a node

Requires Ubuntu/Debian, root or sudo. Installs the Rust binary, creates the `btcpc` system user, and enables auto-update (tracks `stable` branch, checks every 10 min).

```bash
curl -fsSL https://btcpc.net/install.sh | sudo bash
```

After install, edit the service file before starting:

```bash
sudo nano /etc/systemd/system/btcpc-node.service
# Set: BTCPC_ACCOUNT, BTCPC_NODE_ID
```

Start it:

```bash
systemctl enable --now btcpc-node
```

### Docker

```bash
docker run -d \
  -e BTCPC_ACCOUNT=yourname \
  -p 4242:4242 -p 6942:6942 \
  btcpc/node
```

### Running over a VPN (recommended for privacy)

Nodes communicate on port 6942. For privacy, run behind WireGuard:

```ini
# In /etc/systemd/system/btcpc-node.service
Environment="BTCPC_P2P_ANNOUNCE_ADDR=/ip4/10.x.x.x/tcp/6942"
```

Replace `10.x.x.x` with your VPN interface IP.

---

## Earn BTCPC

Every device earns by doing useful work. Rewards are demand-driven — no fixed split. Pools with more activity earn more of the epoch reward.

| Role | What it does | Hardware |
|------|-------------|----------|
| **Clock** | Keeps epoch timing alive | Anything — phone, Pi, laptop |
| **Miner** | AI inference via Ollama | Any computer (GPU earns more) |
| **Sensor** | Reports BLE trackers, environmental data | Android phone, Raspberry Pi, LoRa gateway |
| **Storage** | Hosts BTCPC-FS blobs | Spare SSD or HDD |
| **Service** | Runs apps and APIs for the network | VPS or server |

Multiple roles stack. A Raspberry Pi running BLE scanning + clock earns from two pools simultaneously.

### GPU Mining

```bash
# Install Ollama and pull a model
curl -fsSL https://ollama.ai/install.sh | sh
ollama pull qwen2.5:7b

# Start the node with mining enabled
BTCPC_ACCOUNT=yourname btcpc-node
```

### Raspberry Pi (BLE Tracker + Clock)

```bash
BTCPC_ACCOUNT=yourname \
BTCPC_BLE_TRACKER=true \
btcpc-node
```

The Pi passively scans for AirTags, Android Find My, Tile, and Samsung SmartTags without pairing with any device. Owners of tracked devices pay a subscription fee to receive encrypted sighting data — a share of that fee goes to the observer nodes that reported sightings.

### Phone

Download the [Android APK](https://github.com/shindevlin/btcpc/releases/latest/download/BTCPC-android-release.apk) or open [btcpc.net/app](https://btcpc.net/app) in your browser. Enable sensors and clock from the app UI. First epoch reward arrives within 30 seconds.

### Telegram Wallet

No install needed. Message [@btcpcbot](https://t.me/btcpcbot):

```
/create yourname
```

---

## Use BTCPC in your project

Drop-in replace OpenAI with the BTCPC inference API:

```javascript
const OpenAI = require('openai');
const client = new OpenAI({
  baseURL: 'https://btcpc.net/v1',
  apiKey: process.env.BTCPC_API_KEY
});

const answer = await client.chat.completions.create({
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

Get an API key: create an account via [Telegram bot](https://t.me/btcpcbot), then claim 1 BTCPC free from the faucet: `POST /api/faucet/claim`.

---

## LinkGit

Decentralized git on BTCPC-FS. Repository objects (commits, trees, blobs) stored as content-addressed blobs. Branch and tag refs recorded on-chain. Private repos encrypted to the owner's hide key — no storage node can read the content.

```bash
git remote add origin linkgit://yourname/yourrepo
git push origin main
```

See [docs/LINKGIT_PROTOCOL.md](docs/LINKGIT_PROTOCOL.md) for the full protocol reference.

---

## Freeport Commerce

Sovereign, censorship-resistant marketplace built into the chain. All commerce state (stores, products, orders) flows through the same append-only ledger. Every BTCPC node holds a complete, verifiable copy of the catalog.

- Escrow-protected orders, automatic on delivery
- Digital goods fulfill instantly via BTCPC-FS `delivery_cid`
- Tor onion routing for vendor privacy
- No central marketplace server required

---

## How it works

- 30-second epochs, 42M total supply — fixed forever, like Bitcoin
- Reward pools: calibration-normalized, demand-driven every epoch — no fixed percentages
- Proof of Compute: every token represents real AI inference, sensor coverage, or storage
- All chain state on RocksDB (no separate database required)
- P2P mesh via libp2p — every node is a relay

---

## For agents and developers

See [AGENTS.md](AGENTS.md) for the agent onboarding guide, repo map, and contribution rules.

Copy-paste prompt for Codex, Claude, Cursor, or local agents:

```
You are working on BTCPC (Bitcoin Proof of Compute). Read AGENTS.md,
CONTRIBUTING.md, and README.md first. Use the code-review-graph MCP tools
before Grep or file scanning. Identify the highest-impact change that
improves onboarding, install reliability, or live network proof. Make the
smallest safe patch, run tests, produce a PR summary. Do not change token
economics, genesis constants, or security-sensitive flows unless explicitly
asked.
```

---

## Links

- **Website:** [btcpc.net](https://btcpc.net)
- **Telegram:** [@btcpcbot](https://t.me/btcpcbot)
- **Explorer:** [btcpc.net/dashboard](https://btcpc.net/dashboard)
- **Whitepaper:** [docs/BTCPC_WHITEPAPER.md](docs/BTCPC_WHITEPAPER.md)
- **Reddit:** [r/btcpc](https://reddit.com/r/btcpc)
- **Substack:** [btcpc.substack.com](https://btcpc.substack.com)

## License

MIT

# Bitcoin Proof of Compute (BTCPC)

AI inference on a blockchain. Mine with your GPU. Earn BTCPC. Every token backed by real work.

> **🚫 PULL ONLY — DO NOT START THE NODE YET.**
> Genesis has not launched. The network has no agreed start time.
> Running the node now will produce a chain that cannot sync with anyone else.
> **Wait for a README update that sets `BTCPC_GENESIS_TIMESTAMP` before starting.**
> Pull and build freely — just don't `systemctl enable --now btcpc-node` until then.

> **⚠ Node.js layer deprecated — clean genesis in progress.**
> The canonical chain implementation is now `rust/btcpc-node` (single Rust binary).
> `src/` is retained for reference only and will be removed after genesis cutover.
> See [`rust/btcpc-node/`](rust/btcpc-node/) for the active codebase.

## Stack

- **Rust** — `rust/btcpc-node` — single binary: libp2p networking, RocksDB state, clock consensus, miner, WASM contracts, Axum HTTP API
- **Rust** — `rust/btcpc-contract-sdk` — BSP-20 / BSP-721 smart contract SDK
- **Rust** — `rust/btcpc-contract-runtime` — wasmtime WASM execution sandbox
- Node.js (`src/`) — **deprecated**, retained for reference only
- Ollama — AI inference backend, model-agnostic

## Rust Chain Core Wiring

- `btcpc-chain` listens on `/tmp/btcpc-chain.sock` (override: `BTCPC_CHAIN_SOCK`).
- Set `BTCPC_USE_RUST_CHAIN=true` to route `src/chain/blockStore.js` through Rust IPC (`block_write`, `block_read`, `block_prune`, latest checks).
- Default behavior is safe fallback to file store if IPC is unavailable. Set `BTCPC_RUST_CHAIN_STRICT=true` to fail fast instead.
- IPC timeout is configurable via `BTCPC_CHAIN_IPC_TIMEOUT_MS` (default `5000`).

## Install a node

Requires Ubuntu/Debian, root or sudo. Installs Rust, builds the binary, creates the `btcpc` system user, and enables the auto-update timer (tracks `stable` branch, checks every 10 min).

```bash
curl -fsSL https://raw.githubusercontent.com/shindevlin/btcpc/main/scripts/install.sh | sudo bash
```

After install, edit the service file before starting:

```bash
sudo nano /etc/systemd/system/btcpc-node.service
# Fill in: BTCPC_ACCOUNT, BTCPC_NODE_ID, BTCPC_GENESIS_TIMESTAMP
```

> **Do not start the node until a genesis timestamp is announced.**

### Running over a VPN (recommended for privacy)

Nodes communicate over TCP/UDP on port 6942. For privacy, run your node behind a VPN (WireGuard recommended) so your public IP is never exposed to peers.

1. Bring up your VPN interface (e.g. `wg0`, typically `10.x.x.x`)
2. In `/etc/systemd/system/btcpc-node.service`, set:
   ```
   Environment="BTCPC_P2P_ANNOUNCE_ADDR=/ip4/10.x.x.x/tcp/6942"
   ```
   Replace `10.x.x.x` with your VPN interface IP. The node will advertise this address to peers instead of your real IP.
3. Firewall: allow port 6942 only from your VPN subnet, not the public interface.

For the bootstrap node, we use **Cloudflare Tunnel** so the real IP is never in DNS. To set up a bootstrap tunnel on your machine:

```bash
# Install cloudflared
curl -fsSL https://pkg.cloudflare.com/cloudflare-main.gpg | sudo tee /usr/share/keyrings/cloudflare-main.gpg >/dev/null
echo "deb [signed-by=/usr/share/keyrings/cloudflare-main.gpg] https://pkg.cloudflare.com/cloudflared any main" | sudo tee /etc/apt/sources.list.d/cloudflared.list
sudo apt update && sudo apt install -y cloudflared

# Authenticate and create a tunnel (run once)
cloudflared tunnel login
cloudflared tunnel create btcpc-bootstrap

# Route bootstrap1.btcpc.network through the tunnel
cloudflared tunnel route dns btcpc-bootstrap bootstrap1.btcpc.network

# Create tunnel config at /etc/cloudflared/config.yml:
# tunnel: <tunnel-id>
# credentials-file: /root/.cloudflared/<tunnel-id>.json
# ingress:
#   - hostname: bootstrap1.btcpc.network
#     service: tcp://localhost:6942
#   - service: http_status:404

# Run as a service
cloudflared service install
systemctl enable --now cloudflared
```

With this setup, `bootstrap1.btcpc.network` reaches your node through Cloudflare — your real IP never appears in DNS or peer logs.

## Use BTCPC in your project

```javascript
const BTCPC = require('@btcpc/sdk');
const ai = new BTCPC({ apiKey: process.env.BTCPC_API_KEY });

const answer = await ai.ask({ prompt: 'Explain quantum computing' });
```

Or drop-in replace OpenAI:

```javascript
const OpenAI = require('openai');
const client = new OpenAI({
  baseURL: 'https://btcpc.net/v1',
  apiKey: process.env.BTCPC_API_KEY
});
```

Or curl:

```bash
curl -X POST https://btcpc.net/v1/chat/completions \
  -H "Authorization: Bearer btcpc_your_key" \
  -H "Content-Type: application/json" \
  -d '{"model": "auto", "messages": [{"role": "user", "content": "Hello"}]}'
```

## Get an API key

1. Create an account via [Telegram bot](https://t.me/btcpcbot) or the install wizard
2. Register your project: `POST /api/projects/register`
3. Get 1 BTCPC free from the faucet: `POST /api/faucet/claim`

## Earn BTCPC

Every device earns by doing useful work:

| Role | What it does | Hardware needed |
|------|-------------|-----------------|
| **Miner** | AI inference via Ollama | Any computer (GPU = more earnings) |
| **Clock** | Keeps epoch timing alive | Anything (phone, Pi, laptop) |
| **Storage** | Hosts files for the network | Disk space |
| **Gateway** | Relays IoT sensor data | LoRa gateway (Nebra, RAK, etc.) |
| **Sensor** | Reports real-world data | Any sensor (temp, GPS, air quality) |

Bigger stake = higher reward weight. More useful work = more earnings.

## Commerce

BTCPC Market is a sovereign, censorship-resistant marketplace for hardware, digital goods, and AI compute — built directly into the chain.

**Decentralized catalog.** All commerce state (stores, products, orders) flows through the same append-only ledger as the rest of the chain. Every BTCPC node that replays the ledger holds a complete, verifiable copy of the market catalog. No central marketplace server is required. Catalog reads are served from any node via `GET /api/peer/commerce/stores` and `GET /api/peer/commerce/products`.

**Public access without running a node.** The store frontend (`website/store.html`) is a static file that can be hosted anywhere. `API_BASE` defaults to same-origin for local nodes and is overridable via `?node=` query parameter or `localStorage`. Users without a local node point at any public BTCPC gateway — the gateway serves the catalog from its local ledger, and the data is verifiable on-chain.

**Vendor features:**

- Escrow-protected orders with automatic dispute resolution
- Auto-deliver for digital goods: products with a BTCPC-FS `delivery_cid` fulfill instantly on order placement — zero seller action, trustless delivery
- Flash sales and time-limited pricing
- Shipping account integration: carrier credentials stored on-chain, auto-populated at fulfillment
- Tor onion routing: vendors generate a `.onion` address, register it on-chain, buyers on Tor Browser route through it automatically

**btcpc-market.** The `btcpc-market` Rust service (port 7042) is an optional sidecar that vendors run for full seller operations. Standard BTCPC nodes handle read-only catalog access without it.

## Key management

BTCPC accounts support three separate keys, each scoped to a different privilege level:

| Key | One-line description |
|-----|---------------------|
| **Posting key** | Signs all non-financial chain entries — store ops, product listings, order actions, Q&A |
| **Active key** | Signs token transfers — escrow debit on order placement, escrow release on delivery |
| **Memo key** | Encrypts and signs reputation memos written after a completed trade |

Phase G ships with posting-key-only operations. Active key escrow and memo key reputation are Phase H.

See [Appendix N of the whitepaper](docs/BTCPC_WHITEPAPER.md#appendix-n--key-architecture) for full details on key types, escrow flow, reputation memos, and buyer staking.

## How it works

- 30-second epochs, 42M total supply
- 6-pool rewards: 55% miners, 10% verifiers, 5% clocks, 12% storage, 8% services, 10% IoT
- Proof of Compute: every token represents real AI inference, not wasted energy
- All chain state lives on disk (no database required)
- P2P mesh network — every node is a relay

## Links

- **Website:** [btcpc.net](https://btcpc.net)
- **Telegram:** [@btcpcbot](https://t.me/btcpcbot)
- **Explorer:** [scan.btcpc.net](https://scan.btcpc.net)
- **Whitepaper:** [BTCPC_WHITEPAPER.md](docs/BTCPC_WHITEPAPER.md)
- **SDK:** [sdk/](sdk/)

## License

MIT

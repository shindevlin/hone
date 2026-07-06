# btcpc-clock — Lightweight Clock Node

## What It Does

A clock node participates in BTCPC epoch consensus without mining. No GPU, no MongoDB, no Ollama required. It:

- Connects to the P2P network
- Receives and relays all messages
- Tracks chain state via block files
- Participates in epoch timing consensus
- When eligible: broadcasts EPOCH_START/END

## Requirements

- Node.js 18+
- `ws` npm package (included in btcpc dependencies)
- Network access (WebSocket)
- ~50MB disk (for block files)

**NOT required:** GPU, MongoDB, Ollama, large RAM

## Quick Start

```bash
cd ~/repos/btcpc
npm install --production

HONE_CLOCK_ACCOUNT=josh \
HONE_SEED_PEERS=ws://100.122.145.60:6942 \
P2P_PORT=6943 \
node bin/btcpc-clock
```

## iPad Setup (a-Shell)

1. Install **a-Shell** from the App Store (free)
2. Open a-Shell and run:

```bash
# Install dependencies
pkg install nodejs lg2

# Clone the repo
lg2 clone https://github.com/shindevlin/btcpc.git
cd btcpc

# Install npm packages
npm install --production

# Create environment file
cat > .env.clock << 'EOF'
HONE_CLOCK_ACCOUNT=josh
HONE_SEED_PEERS=ws://100.122.145.60:6942
P2P_PORT=6943
HONE_RELAY_URL=wss://btcpc-relay.shindevlin.workers.dev/ws
EOF

# Run the clock node
source .env.clock && node bin/btcpc-clock
```

## Permission Tiers

| Tier | Requirements | Epoch Consensus |
|------|-------------|-----------------|
| **Permissioned** | Approved by genesis operator | Immediate eligibility |
| **Permissionless** | 100+ BTCPC staked | Eligible after staking |
| **Genesis** | shindevlin (always eligible) | Fallback authority |

## Registering as a Clock Node

To participate in epoch consensus, register on the permanent ledger:

```bash
# Via CLI (when available)
node bin/btcpc-cli node register --type clock --account josh

# Or via API
curl -X POST http://localhost:3000/api/node/register \
  -H "Authorization: Bearer <jwt>" \
  -d '{"type": "clock"}'
```

## How Epoch Consensus Works

1. Every node independently computes epoch boundaries from genesis timestamp
2. When the clock says "new epoch", any eligible node broadcasts EPOCH_START
3. The network accepts the first valid EPOCH_START from a registered node
4. Mining happens during the epoch (miners submit proofs)
5. At epoch end, any eligible node with mining proofs can finalize
6. If the primary misses, other eligible nodes step in

No single point of failure. If shindevlin goes offline, other clock nodes keep the chain running.

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `HONE_CLOCK_ACCOUNT` | `clock-<random>` | Account name for this clock node |
| `P2P_PORT` | `6943` | WebSocket server port |
| `HONE_SEED_PEERS` | (none) | Comma-separated seed peer addresses |
| `HONE_RELAY_URL` | Cloudflare relay | NAT traversal relay |
| `HONE_NODE_ID` | (auto-generated) | Persistent node identity |

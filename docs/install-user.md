# Install: User Node

Lightweight node for wallet operations, submitting inference requests, and trading BTCPC. No GPU required.

## Requirements

- **Node.js**: 20+
- **MongoDB**: 7+ (Docker or native) — or connect to a remote node's MongoDB

## Step 1: Clone and Install

```bash
git clone https://github.com/shindevlin/btcpc.git
cd btcpc
npm install
```

## Step 2: Configure

```bash
cp .env.example .env
```

Edit `.env`:
```
MONGODB_URI=mongodb://root:example@localhost:27017/btcpc?authSource=admin
P2P_PORT=6942
HONE_SEED_PEERS=ws://100.90.146.17:6942
JWT_SECRET=<generate with: openssl rand -hex 32>
```

If connecting to an existing network (no local MongoDB):
```
MONGODB_URI=mongodb://root:example@<network-node-ip>:27017/btcpc?authSource=admin
```

## Step 3: Start API Server

```bash
npm start
```

This starts the API on port 3000. You can now:
- Create accounts
- Check balances
- Transfer BTCPC
- Submit inference requests
- Stake tokens

## Step 4: Create an Account

```bash
curl -X POST http://localhost:3000/api/user/register \
  -H "Content-Type: application/json" \
  -d '{"username": "alice", "email": "alice@example.com", "password": "your-password"}'
```

## Step 5: Submit Inference Requests

Using the client SDK:
```javascript
const { BTCPCClient } = require('./src/inference/client');

const client = new BTCPCClient({
  nodeUrl: 'http://localhost:3000',
  apiKey: 'your-jwt-token',
});

const result = await client.inference({
  model: 'qwen3.5:27b',
  prompt: 'Explain quantum computing',
});
console.log(result.text);
```

Your prompt is tokenized locally, encrypted, and sent to the network. No node sees your plaintext.

## Telegram Bot

Message [@btcpcbot](https://t.me/btcpcbot):
```
/link alice
/balance
/history
```

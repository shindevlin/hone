# Bitcoin Proof of Compute (BTCPC)

Decentralized AI inference backed by blockchain. Use GPU compute from the BTCPC network in your project — pay with BTCPC tokens, every request is verified on-chain.

**42,000,000 total supply. Every token backed by real GPU work.**

## Use BTCPC in Your Project

Any project can plug into BTCPC for AI inference. OpenAI-compatible API — swap your base URL and go.

### Option A: Drop-in OpenAI replacement

```javascript
const OpenAI = require('openai');

const client = new OpenAI({
  baseURL: 'https://api.btcpc.network/v1',
  apiKey: 'btcpc_your_key_here'
});

const res = await client.chat.completions.create({
  model: 'qwen3.5:27b',
  messages: [{ role: 'user', content: 'Explain quantum computing' }]
});
```

### Option B: BTCPC SDK

```bash
npm install @btcpc/sdk
```

```javascript
const BTCPC = require('@btcpc/sdk');
const ai = new BTCPC({ apiKey: 'btcpc_your_key_here' });

// Simple
const answer = await ai.ask({ prompt: 'Explain quantum computing' });

// Full control
const res = await ai.chat({
  model: 'dolphin-llama3:8b',    // request any model on the network
  messages: [
    { role: 'system', content: 'You are a creative writing assistant.' },
    { role: 'user', content: 'Write a short story about a robot.' }
  ],
  temperature: 0.9,
  maxTokens: 2048
});

console.log(res.choices[0].message.content);
console.log(`Cost: ${res.btcpc.cost} BTCPC (${res.btcpc.tokens_per_btcpc} tokens/BTCPC)`);
```

### Option C: Direct API (curl, any language)

```bash
curl -X POST https://api.btcpc.network/v1/chat/completions \
  -H "Authorization: Bearer btcpc_your_key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "qwen3.5:27b",
    "messages": [{"role": "user", "content": "Hello"}]
  }'
```

### Option D: Feed your own AI

Already running your own agent, bot, or AI tool? Point it at BTCPC:

```bash
# OpenClaw / any OpenAI-compatible tool
export OPENAI_BASE_URL=https://api.btcpc.network/v1
export OPENAI_API_KEY=btcpc_your_key_here

# Your tool now uses BTCPC for inference — no code changes needed
```

Any tool that supports a custom OpenAI base URL works out of the box: LangChain, AutoGPT, OpenClaw, Continue, Cursor, etc.

## Get an API Key

1. **Register your repo:**
   ```bash
   curl -X POST https://api.btcpc.network/api/projects/register \
     -H "Authorization: Bearer YOUR_JWT" \
     -H "Content-Type: application/json" \
     -d '{"repoUrl": "https://github.com/you/your-project"}'
   ```

2. **Verify ownership** — add the `.btcpc` file (contains your wallet address) to your repo root, push, then:
   ```bash
   curl -X POST https://api.btcpc.network/api/projects/verify \
     -H "Authorization: Bearer btcpc_your_key"
   ```

3. **Fund your wallet** — get BTCPC via the faucet (`/claim`), mining, or transfer.

4. **Start using inference** — every call deducts from your project wallet at dynamic rates.

## Pricing

Dynamic — adjusts with network load and model size.

| Model Weight | Example Models | 1 BTCPC buys (idle) |
|---|---|---|
| 1.0x | phi3, gemma2:2b | ~2000 tokens |
| 2.0x | llama3:8b, deepseek-r1:8b | ~1000 tokens |
| 4.0x | qwen3.5:27b, mixtral | ~500 tokens |
| 8.0x | llama3:70b | ~250 tokens |
| 16.0x | qwen2.5:72b | ~125 tokens |

Check live rates: `GET /v1/pricing?model=qwen3.5:27b`

Busier network = higher price. Idle network = cheaper. [Pricing details](docs/getting-started.md)

## Browse Available Models

```bash
# See what models miners are running
curl https://api.btcpc.network/v1/network/models
```

Request a model nobody has? The network broadcasts demand to all miners — incentivizing them to pull it and earn.

## Get Your First Tokens

```bash
# Via API (after login)
curl -X POST https://api.btcpc.network/api/faucet/claim \
  -H "Authorization: Bearer YOUR_JWT"
```

Or via Telegram: `/claim` (after `/link yourname`)

Need more? Email **shin@btcpc.network**

## Run a Miner (Earn BTCPC)

Provide GPU compute to the network and earn BTCPC every epoch.

```bash
git clone https://github.com/shindevlin/btcpc.git
cd btcpc && npm install
cp .env.example .env
node bin/btcpc-mine
```

**Requirements:** Node.js 20+, MongoDB, Ollama, NVIDIA GPU (8GB+ VRAM)

Bigger models = higher weight = more BTCPC per epoch. See [Mining Guide](docs/install-miner.md).

## Architecture

```
┌──────────────────┐     ┌──────────────────┐
│  YOUR PROJECT    │     │  BTCPC NETWORK   │
│                  │     │                  │
│  @btcpc/sdk      │────→│  API Gateway     │
│  or OpenAI SDK   │     │  /v1/chat/...    │
│  or curl         │     │                  │
│  or any AI tool  │←────│  Dynamic pricing │
│                  │     │  Work proofs     │
└──────────────────┘     │  On-chain billing│
                         │                  │
                         │  ┌────────────┐  │
                         │  │ Miners     │  │
                         │  │ (Ollama)   │  │
                         │  │ GPU compute│  │
                         │  └────────────┘  │
                         └──────────────────┘
```

## Telegram Bot

[@btcpcbot](https://t.me/btcpcbot) — claim tokens, check balance, browse models, submit inference, mining stats

## Easiest Install

For a normal user on Ubuntu or Debian, use one command:

```bash
curl -fsSL https://btcpc.net/install.sh | bash -s -- <your-username>
```

What it does:
- installs Node.js and Docker if needed
- starts MongoDB automatically in Docker
- clones BTCPC
- installs dependencies
- starts BTCPC in wallet/explorer/clock mode

After it finishes, open:

```text
http://localhost:4242
```

If you want mining later, run:

```bash
cd ~/btcpc && BTCPC_MINER=<your-username> node bin/btcpc-setup
```

## Community & Contact

- Telegram Group: [t.me/btcpcnetwork](https://t.me/btcpcnetwork)
- Telegram Bot: [@btcpcbot](https://t.me/btcpcbot)
- Email: shin@btcpc.network
- Issues: [github.com/shindevlin/btcpc/issues](https://github.com/shindevlin/btcpc/issues)

## Documentation

- [Integration Guide](docs/integration.md) — connect your project to BTCPC
- [Getting Started](docs/getting-started.md) — accounts, tokens, first inference
- [Mining Guide](docs/install-miner.md) — earn BTCPC with your GPU
- [API Reference](docs/api.md) — full endpoint docs
- [SDK](sdk/) — npm package source and docs
- [Whitepaper](docs/BTCPC_WHITEPAPER.md) — Proof of Compute protocol
- [Architecture](docs/L2_ARCHITECTURE.md) — cross-chain design

## Project Status

The repository is beyond scaffold stage. Core API, wallets, ledger, mining, P2P, inference, bots, explorer, and cross-chain groundwork are present in code.

What is still in progress is verification and cleanup:
- automated testing is still being built out
- some roadmap items are partial or started but not production-complete
- docs and tracker files are being aligned with the actual implementation state

See [docs/PROJECT_STATE.md](docs/PROJECT_STATE.md), [docs/EXECUTION_PLAN.md](docs/EXECUTION_PLAN.md), and [tasks/tasks.md](tasks/tasks.md) for the current state and next priorities.

## Testing

Run the current unit test suite with:

```bash
npm test -- --runInBand
```

Current baseline coverage focuses on:
- authentication compatibility and login behavior
- wallet controller safety paths
- ledger accounting primitives
- pricing calculations

The suite is intentionally unit-focused for now. End-to-end and miner/inference integration coverage still need to be added.

## License

MIT

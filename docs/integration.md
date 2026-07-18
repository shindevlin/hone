# Integration Guide — Use HONE Inference in Your Project

HONE is an OpenAI-compatible inference API backed by a decentralized GPU mining network. Any project that calls an LLM can use HONE instead of OpenAI, Anthropic, or local Ollama.

## Why HONE

- Every inference call is verified on-chain with a cryptographic work proof
- Dynamic pricing — cheaper when the network is idle, no monthly subscription
- Model choice — request any model miners are running (or signal demand for new ones)
- Privacy — prompts are hashed, never stored in plaintext on-chain
- Your project earns a verified wallet on the HONE blockchain

## Step 1: Register Your Repository

```bash
# Login to get a JWT
curl -X POST https://api.honemesh.network/api/user/login \
  -H "Content-Type: application/json" \
  -d '{"email": "you@example.com", "password": "your-password"}'

# Register your GitHub repo
curl -X POST https://api.honemesh.network/api/projects/register \
  -H "Authorization: Bearer YOUR_JWT" \
  -H "Content-Type: application/json" \
  -d '{"repoUrl": "https://github.com/you/your-project"}'
```

You'll get back:
```json
{
  "apiKey": "hone_abc123...",
  "walletAddress": "hone_proj_def456...",
  "next_steps": [
    "Add a .hone file to your repo root containing: hone_proj_def456...",
    "Push it to your default branch",
    "Call POST /api/projects/verify"
  ]
}
```

## Step 2: Verify Ownership

Create a `.hone` file in your repo root:
```bash
echo "hone_proj_def456..." > .hone
git add .hone && git commit -m "Add HONE project wallet" && git push
```

Then verify:
```bash
curl -X POST https://api.honemesh.network/api/projects/verify \
  -H "Authorization: Bearer hone_your_key"
```

## Step 3: Fund Your Wallet

Get HONE tokens into your project wallet:

```bash
# Claim faucet (1 HONE, one-time)
curl -X POST https://api.honemesh.network/api/faucet/claim \
  -H "Authorization: Bearer YOUR_JWT"

# Transfer from your personal wallet to project
curl -X POST https://api.honemesh.network/api/projects/fund \
  -H "Authorization: Bearer YOUR_JWT" \
  -H "Content-Type: application/json" \
  -d '{"walletAddress": "hone_proj_def456...", "amount": 10}'
```

Or ask for tokens: **shindevlin@proton.me**

## Step 4: Use Inference

### With @hone/sdk (Node.js)

```bash
npm install @hone/sdk
```

```javascript
const HONE = require('@hone/sdk');

const ai = new HONE({
  apiKey: process.env.HONE_API_KEY,
  baseUrl: 'https://api.honemesh.network'  // or http://localhost:3000 for local
});

// Simple prompt → text
const answer = await ai.ask({ prompt: 'What is proof of compute?' });

// Full chat (OpenAI format)
const res = await ai.chat({
  model: 'qwen3.5:27b',
  messages: [
    { role: 'system', content: 'You are a helpful assistant.' },
    { role: 'user', content: 'Explain HONE mining.' }
  ],
  temperature: 0.7,
  maxTokens: 1024
});

// Check pricing before heavy workloads
const rates = await ai.pricing('qwen3.5:27b');
console.log(`1 HONE = ${rates.tokens_per_hone} tokens at current load`);

// See what models are available
const { available, wanted } = await ai.networkModels();
```

### With OpenAI SDK (any language)

**Node.js:**
```javascript
const OpenAI = require('openai');
const client = new OpenAI({
  baseURL: 'https://api.honemesh.network/v1',
  apiKey: 'hone_your_key'
});

const res = await client.chat.completions.create({
  model: 'qwen3.5:27b',
  messages: [{ role: 'user', content: 'Hello' }]
});
```

**Python:**
```python
from openai import OpenAI
client = OpenAI(
    base_url="https://api.honemesh.network/v1",
    api_key="hone_your_key"
)

res = client.chat.completions.create(
    model="qwen3.5:27b",
    messages=[{"role": "user", "content": "Hello"}]
)
```

### With any OpenAI-compatible tool

Set environment variables — no code changes needed:

```bash
export OPENAI_BASE_URL=https://api.honemesh.network/v1
export OPENAI_API_KEY=hone_your_key
```

This works with: LangChain, AutoGPT, OpenClaw, Continue, Cursor, Aider, and any tool that reads `OPENAI_BASE_URL`.

### With curl (any language)

```bash
curl -X POST https://api.honemesh.network/v1/chat/completions \
  -H "Authorization: Bearer hone_your_key" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "qwen3.5:27b",
    "messages": [{"role": "user", "content": "Hello"}]
  }'
```

## Choosing a Model

Check what's available:
```bash
curl https://api.honemesh.network/v1/network/models
```

Response:
```json
{
  "available": [
    { "model": "qwen3.5:27b", "miners": 3, "cost_per_token": 0.002, "model_weight": 4.0 },
    { "model": "dolphin-llama3:8b", "miners": 1, "cost_per_token": 0.001, "model_weight": 2.0 }
  ],
  "wanted": [
    { "model": "llama3:70b", "requests": 12 }
  ]
}
```

- **available** — models you can use right now
- **wanted** — models users have requested but no miner has yet (demand signal)

If you request a model nobody has, the network broadcasts demand to all miners. Miners with capable hardware are incentivized to pull it.

## Pricing

Dynamic based on two factors:

1. **Network load** — busier = more expensive, idle = cheaper
2. **Model size** — bigger models cost more per token

```bash
# Check rate for a specific model
curl "https://api.honemesh.network/v1/pricing?model=qwen3.5:27b"
```

```json
{
  "model": "qwen3.5:27b",
  "tokens_per_hone": 500,
  "cost_per_token": 0.002,
  "load_multiplier": 0.5,
  "model_weight": 4.0,
  "total_multiplier": 2.0,
  "network_load": 0.0
}
```

Every inference response includes billing:
```json
{
  "hone": {
    "cost": 0.512,
    "tokens_per_hone": 500,
    "model_weight": 4.0,
    "remaining_balance": 9.488
  }
}
```

## Check Your Balance

```bash
curl https://api.honemesh.network/api/projects/me \
  -H "Authorization: Bearer hone_your_key"
```

```json
{
  "name": "you/your-project",
  "balance": 9.488,
  "totalSpent": 0.512,
  "totalRequests": 3,
  "verified": true
}
```

## Example: Continuous Content Generation

```javascript
const HONE = require('@hone/sdk');
const ai = new HONE({ apiKey: process.env.HONE_API_KEY });

async function generateLoop() {
  while (true) {
    // Check balance before generating
    const { balance } = await ai.project();
    if (balance < 1) {
      console.log('Low balance, pausing');
      await new Promise(r => setTimeout(r, 60000));
      continue;
    }

    const res = await ai.chat({
      model: 'dolphin-llama3:8b',
      messages: [{ role: 'user', content: 'Write a short story.' }],
      maxTokens: 2048,
      temperature: 0.85
    });

    console.log(`Generated: ${res.usage.completion_tokens} tokens, cost: ${res.hone.cost} HONE`);
    // Save res.choices[0].message.content to your DB

    await new Promise(r => setTimeout(r, 10000));
  }
}
```

## Endpoints Reference

| Method | Path | Auth | Description |
|---|---|---|---|
| POST | `/api/projects/register` | JWT | Register a GitHub repo |
| POST | `/api/projects/verify` | API key | Verify repo ownership |
| GET | `/api/projects/me` | API key | Project info + balance |
| POST | `/api/projects/fund` | JWT | Send HONE to a project |
| POST | `/v1/chat/completions` | API key | OpenAI-compatible inference |
| GET | `/v1/models` | API key | Local node models |
| GET | `/v1/network/models` | API key | All models across network |
| GET | `/v1/pricing?model=` | API key | Dynamic pricing |
| POST | `/api/faucet/claim` | JWT | Claim 1 free HONE |

## Support

- Telegram: [t.me/honenetwork](https://t.me/honenetwork)
- Bot: [@honebot](https://t.me/honebot)
- Email: shindevlin@proton.me
- Issues: [github.com/shindevlin/hone/issues](https://github.com/shindevlin/hone/issues)

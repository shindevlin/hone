# @btcpc/sdk

Use AI inference powered by Bitcoin Proof of Compute. Every request is backed by verified GPU work on the BTCPC blockchain.

## Install

```bash
npm install @btcpc/sdk
```

## Quick Start

```javascript
const BTCPC = require('@btcpc/sdk');

const ai = new BTCPC({ apiKey: 'btcpc_your_key_here' });

// Simple prompt
const answer = await ai.ask({ prompt: 'Explain quantum computing in one paragraph' });
console.log(answer);

// Full chat with message history
const res = await ai.chat({
  messages: [
    { role: 'system', content: 'You are a helpful assistant.' },
    { role: 'user', content: 'What is proof of compute?' }
  ],
  model: 'qwen3.5:27b',
  temperature: 0.7
});
console.log(res.choices[0].message.content);
console.log(`Cost: ${res.btcpc.cost} BTCPC`);
```

## OpenAI Drop-in

Already using the OpenAI SDK? Just change the base URL:

```javascript
const OpenAI = require('openai');

const client = new OpenAI({
  baseURL: 'https://api.btcpc.network/v1',
  apiKey: 'btcpc_your_key_here'
});

const res = await client.chat.completions.create({
  model: 'qwen3.5:27b',
  messages: [{ role: 'user', content: 'Hello' }]
});
```

## Get an API Key

1. Register your GitHub repo:
   ```bash
   curl -X POST https://api.btcpc.network/api/projects/register \
     -H "Authorization: Bearer YOUR_JWT" \
     -H "Content-Type: application/json" \
     -d '{"repoUrl": "https://github.com/you/your-repo"}'
   ```

2. Add the `.btcpc` file to your repo root (contains your wallet address)

3. Verify:
   ```bash
   curl -X POST https://api.btcpc.network/api/projects/verify \
     -H "Authorization: Bearer btcpc_your_key"
   ```

4. Fund your project wallet with BTCPC tokens

## API

### `new BTCPC({ apiKey, baseUrl? })`
Create a client. `baseUrl` defaults to `https://api.btcpc.network`.

### `ai.ask({ prompt, model?, maxTokens?, temperature? })`
Returns just the response text (string).

### `ai.chat({ messages, model?, maxTokens?, temperature? })`
Full OpenAI-compatible chat completion. Returns the complete response object including `btcpc.cost`, `btcpc.proof_hash`, and `usage`.

### `ai.models()`
List available models.

### `ai.project()`
Check your project balance and usage stats.

## Pricing

0.001 BTCPC per completion token. Larger models earn more for miners but cost the same for you.

## Links

- GitHub: [shindevlin/btcpc](https://github.com/shindevlin/btcpc)
- Telegram: [t.me/btcpcnetwork](https://t.me/btcpcnetwork)
- Email: shindevlin@proton.me

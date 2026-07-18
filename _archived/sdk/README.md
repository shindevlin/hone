# @hone/sdk

Use AI inference powered by Bitcoin Proof of Compute. Every request is backed by verified GPU work on the HONE blockchain.

## Install

```bash
npm install @hone/sdk
```

## Quick Start

```javascript
const HONE = require('@hone/sdk');

const ai = new HONE({ apiKey: 'hone_your_key_here' });

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
console.log(`Cost: ${res.hone.cost} HONE`);
```

## OpenAI Drop-in

Already using the OpenAI SDK? Just change the base URL:

```javascript
const OpenAI = require('openai');

const client = new OpenAI({
  baseURL: 'https://api.hone.network/v1',
  apiKey: 'hone_your_key_here'
});

const res = await client.chat.completions.create({
  model: 'qwen3.5:27b',
  messages: [{ role: 'user', content: 'Hello' }]
});
```

## Get an API Key

1. Register your GitHub repo:
   ```bash
   curl -X POST https://api.hone.network/api/projects/register \
     -H "Authorization: Bearer YOUR_JWT" \
     -H "Content-Type: application/json" \
     -d '{"repoUrl": "https://github.com/you/your-repo"}'
   ```

2. Add the `.hone` file to your repo root (contains your wallet address)

3. Verify:
   ```bash
   curl -X POST https://api.hone.network/api/projects/verify \
     -H "Authorization: Bearer hone_your_key"
   ```

4. Fund your project wallet with HONE tokens

## API

### `new HONE({ apiKey, baseUrl? })`
Create a client. `baseUrl` defaults to `https://api.hone.network`.

### `ai.ask({ prompt, model?, maxTokens?, temperature? })`
Returns just the response text (string).

### `ai.chat({ messages, model?, maxTokens?, temperature? })`
Full OpenAI-compatible chat completion. Returns the complete response object including `hone.cost`, `hone.proof_hash`, and `usage`.

### `ai.models()`
List available models.

### `ai.project()`
Check your project balance and usage stats.

## Pricing

0.001 HONE per completion token. Larger models earn more for miners but cost the same for you.

## Links

- GitHub: [shindevlin/hone](https://github.com/shindevlin/hone)
- Telegram: [t.me/honenetwork](https://t.me/honenetwork)
- Email: shindevlin@proton.me

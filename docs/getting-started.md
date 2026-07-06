# Getting Started with BTCPC

## Prerequisites
- Node.js (v20+ recommended)
- npm
- MongoDB 7+ (Docker or native)
- Ollama (if mining — not needed for user nodes)

## Installation

```bash
git clone https://github.com/shindevlin/btcpc.git
cd btcpc
npm install
cp .env.example .env   # edit with your config
```

## Start the Node

```bash
npm start              # API on :3000
```

To mine (requires GPU):
```bash
node bin/btcpc-mine
```

## Create an Account

```bash
curl -X POST http://localhost:3000/api/user/register \
  -H "Content-Type: application/json" \
  -d '{"username": "yourname", "email": "you@example.com", "password": "your-password"}'
```

## Get Your First Tokens

New accounts start with 0 BTCPC. There are three ways to get tokens:

1. **Faucet (easiest)** — claim 1 free BTCPC per account:
   ```bash
   # Via API (requires JWT from login)
   curl -X POST http://localhost:3000/api/faucet/claim \
     -H "Authorization: Bearer YOUR_JWT_TOKEN"
   ```
   Or via Telegram: `/claim` (after linking your account with `/link yourname`)

2. **Mine** — run a GPU miner node and earn BTCPC per epoch. See [install-miner.md](install-miner.md).

3. **Request more** — email **shindevlin@proton.me** with your username for additional tokens.

## Link Telegram (Optional)

Use [@btcpcbot](https://t.me/btcpcbot) to check balances, mining stats, and submit AI tasks:

1. Message the bot: `/link yourname`
2. Claim starter tokens: `/claim`
3. Check balance: `/balance`
4. Submit an AI task: just type any message

## Run AI Tasks

Once you have tokens, submit AI tasks to the network:

```bash
curl -X POST http://localhost:3000/v1/chat/completions \
  -H "Authorization: Bearer YOUR_JWT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"model": "qwen3.5:27b", "messages": [{"role": "user", "content": "Hello"}]}'
```

Cost: 0.001 BTCPC per completion token.

## What's Next

- [Mining Guide](install-miner.md) — earn BTCPC with your GPU
- [API Reference](api.md) — full endpoint docs
- [Whitepaper](HONE_WHITEPAPER.md) — how Proof of Compute works
- [FAQ](faq.md) — common issues

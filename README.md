# Bitcoin Proof of Compute (BTCPC)

AI inference on a blockchain. Mine with your GPU. Earn BTCPC. Every token backed by real work.

## Install (one command)

```bash
curl -fsSL https://btcpc.net/install.sh | bash
```

That's it. It installs Node.js, Ollama, NVIDIA drivers (if you have a GPU), clones BTCPC, and starts a setup wizard.

**Phone (Termux):**
```bash
curl -fsSL https://btcpc.net/install-termux.sh | bash
```

**Windows:** Download [btcpc-start.bat](https://btcpc.net/install) or run in WSL.

## Install with AI help

Don't want to touch a terminal? Open Claude, ChatGPT, or any AI assistant and paste:

> Install BTCPC on my computer. Run this command and help me through any errors:
> `curl -fsSL https://btcpc.net/install.sh | bash`
> If there are GPU/CUDA issues, fix them. If Node.js fails, try nvm.
> After install, run `node bin/btcpc-all` to start all roles.

The AI will handle everything — installation, troubleshooting, configuration, starting your node.

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

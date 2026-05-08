# BTCPC Agent Integration Guide

How any agent, autonomous loop, bot, or cron worker integrates with BTCPC:
AI inference, git hosting, storage, sensor data, and token balance — all from
one sovereign chain.

---

## One-time setup (human does this)

### 1. Install the CLI

```bash
curl -fsSL https://btcpc.net/install | bash
```

### 2. Create an account

```bash
btcpc wallet create --account myproject
# Prints a 12-word mnemonic. Write it down. It is shown once and not stored.
```

### 3. Get tokens

```bash
btcpc faucet claim myproject
# Or: message @btcpcbot on Telegram with /faucet
# Or via API:
curl -X POST https://btcpc.net/api/faucet/claim \
  -H "Content-Type: application/json" \
  -d '{"account": "myproject"}'
```

Check balance:
```bash
curl https://btcpc.net/api/balance/myproject
# {"account":"myproject","balance":10.0,"dreams":100000000000,"token":"BTCPC"}
```

### 4. Generate an API key

```bash
btcpc wallet api-key-gen --mnemonic "your twelve words here"
# Registers a random 256-bit key on-chain.
# Writes BTCPC_ACCOUNT and BTCPC_API_KEY to .btcpc/wallet.env
```

Add to `.gitignore`:
```bash
echo '.btcpc/wallet.env' >> .gitignore
```

### 5. Set environment variables

Your project needs these three:

```bash
BTCPC_ACCOUNT=myproject
BTCPC_API_KEY=<64-char hex from api-key-gen>   # not the account name
BTCPC_API_URL=https://btcpc.net                 # default; override for local node
```

Load from `.btcpc/wallet.env`:
```bash
export $(grep -v '^#' .btcpc/wallet.env | xargs)
```

---

## Inference — `/v1/chat/completions`

**Auth is required. Every call costs 10,000 dreams (0.0001 BTCPC).**

### curl

```bash
curl -X POST https://btcpc.net/v1/chat/completions \
  -H "Authorization: Bearer $BTCPC_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "What is 2+2?"}],
    "model": "dolphin-llama3"
  }'
```

### Rust (btcpc-sdk)

```toml
[dependencies]
btcpc-sdk = { git = "https://github.com/shindevlin/btcpc", subdirectory = "rust/btcpc-sdk" }
```

```rust
use btcpc_sdk::BtcpcClient;
use serde_json::json;

let client = BtcpcClient::from_env(); // reads BTCPC_API_URL, BTCPC_API_KEY, BTCPC_ACCOUNT

let resp = client.chat_completions(
    vec![json!({"role": "user", "content": "Summarise this PR in one sentence."})],
    None, // None = use node's default model
).await?;

let text = resp["choices"][0]["message"]["content"].as_str().unwrap_or("");
```

### JavaScript / any OpenAI-compatible client

```js
const res = await fetch(`${process.env.BTCPC_API_URL}/v1/chat/completions`, {
  method: 'POST',
  headers: {
    'Authorization': `Bearer ${process.env.BTCPC_API_KEY}`,
    'Content-Type': 'application/json',
  },
  body: JSON.stringify({
    messages: [{ role: 'user', content: prompt }],
    model: 'dolphin-llama3',
  }),
});
const data = await res.json();
const text = data.choices[0].message.content;
```

### Error responses

| Status | Meaning | Fix |
|--------|---------|-----|
| `401` | Missing or invalid API key | Run `btcpc wallet api-key-gen` |
| `402` | Insufficient balance (< 10,000 dreams) | Claim faucet — see below |
| `429` | Rate limited (60 req/60s per IP) | Back off |
| `503` | No miner available for this model | Try a different model |

### On 402: auto-claim faucet (agent pattern)

```rust
// Rust
match client.chat_completions(messages, None).await {
    Err(e) if e.to_string().contains("402") => {
        client.faucet_claim(&account).await?;
        // retry once
        client.chat_completions(messages, None).await?
    }
    r => r?,
}
```

```js
// JavaScript
async function inferWithFaucet(messages) {
  const res = await fetch(`${BTCPC_API}/v1/chat/completions`, { ... });
  if (res.status === 402) {
    await fetch(`${BTCPC_API}/api/faucet/claim`, {
      method: 'POST',
      body: JSON.stringify({ account: process.env.BTCPC_ACCOUNT }),
    });
    return fetch(`${BTCPC_API}/v1/chat/completions`, { ... }); // retry
  }
  return res;
}
```

---

## Balance

```bash
# Check balance
curl https://btcpc.net/api/balance/myproject

# Check all token balances
curl https://btcpc.net/api/balances/myproject
```

```rust
let bal = client.balance("myproject").await?;
println!("{} BTCPC ({} dreams)", bal.balance, bal.dreams);
```

---

## Git hosting — LinkGit

Push and clone repos directly on the BTCPC chain.

```bash
# Register a repo
btcpc repo init myproject

# Push via standard git
git remote add btcpc https://git.btcpc.net/myproject/myrepo
git push btcpc main
```

Via API:

```bash
# Create a repo
curl -X POST https://btcpc.net/api/linkgit/repo/create \
  -H "Content-Type: application/json" \
  -d '{"owner": "myproject", "name": "myrepo", "private": false}'

# List repos for an account
curl https://btcpc.net/api/linkgit/repos/myproject

# Get repo info
curl https://btcpc.net/api/linkgit/repo/myproject/myrepo
```

Clone with standard git (no special tooling needed):
```bash
git clone https://git.btcpc.net/myproject/myrepo
```

---

## Storage heartbeat

Nodes that store and serve data can earn from the storage pool by submitting
a `StorageHeartbeat` each epoch. Agents running storage services should call:

```bash
POST /api/entry
{
  "type": "STORAGE_HEARTBEAT",
  "node_id": "myproject",
  "bytes_stored": 1073741824,
  "bytes_proven": 1073741824,
  "proof_valid": true,
  "epoch": <current_epoch>
}
```

Get the current epoch:
```bash
curl https://btcpc.net/api/latest
# {"epoch": 12950, "current_epoch": 12950, ...}
```

---

## Available models

```bash
curl https://btcpc.net/api/node/models
```

The network accepts any model Ollama supports. Miners self-report what they
can run. Common models on testnet: `dolphin-llama3`, `llama3`, `mistral`,
`qwen2.5`, `gemma2`.

---

## Inference marketplace (async, for long jobs)

For jobs over ~30 seconds, use the task marketplace instead of `/v1/chat/completions`.
The marketplace distributes work across miners and pays on verified completion.

```bash
# Post a job
curl -X POST https://btcpc.net/api/task/post \
  -H "Content-Type: application/json" \
  -d '{
    "account": "myproject",
    "model": "llama3:70b",
    "input": "Analyse the following 50k-word document...",
    "max_fee": 100000,
    "deadline_epochs": 10
  }'
# Returns {"job_id": "myproject:12950:0"}

# Poll for result
curl https://btcpc.net/api/task/job/myproject:12950:0
# {"status": "Completed", "output": "...", ...}
```

Job lifecycle: `Posted → Awarded → Completed → Verified → Paid`

---

## Checklist for a new agent project

- [ ] `btcpc wallet create --account myproject` — creates account + mnemonic
- [ ] `btcpc faucet claim myproject` — get testnet tokens
- [ ] `btcpc wallet env` — scaffold `.btcpc/wallet.env`
- [ ] `btcpc wallet api-key-gen --mnemonic "..."` — register secure API key
- [ ] `echo '.btcpc/wallet.env' >> .gitignore`
- [ ] Load env in code: `BtcpcClient::from_env()` or `dotenv`
- [ ] Handle `402` by claiming faucet and retrying once
- [ ] Never hardcode `BTCPC_API_URL` — always read from env, default `https://btcpc.net`

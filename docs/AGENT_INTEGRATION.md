# BTCPC Agent Integration Guide

Connect any project or AI agent to the BTCPC chain: AI inference, git hosting,
storage, and a token balance — all from one sovereign account.

Web version: https://btcpc.net/integrate

---

## One-time setup

### Step 1 — Install the CLI

```bash
curl -fsSL https://btcpc.net/install | bash
btcpc --version
```

### Step 2 — Check for an existing wallet FIRST

**Do not create a new wallet if one already exists.** You will lose access to
your existing account and any tokens in it.

```bash
# Any of these means you already have a wallet:
ls ~/.btcpc/wallet.json 2>/dev/null && echo EXISTS
ls .btcpc/wallet.env   2>/dev/null && echo EXISTS
echo $BTCPC_ACCOUNT
```

If a wallet exists, skip to Step 5.

### Step 3 — Create your account (only if none exists)

```bash
btcpc wallet create --account myproject
# Prints a 12-word mnemonic. Write it down — shown once, never stored.
```

### Step 4 — Register your signing key (only if new account)

```bash
btcpc key generate
btcpc key register --account myproject --role posting
```

Generates `~/.btcpc/key.json` and registers its public key on-chain.

### Step 5 — Check balance and claim tokens

```bash
curl https://btcpc.net/api/balance/myproject
# {"account":"myproject","balance":10.0,"dreams":100000000000,"token":"BTCPC"}
# 1 BTCPC = 10,000,000,000 dreams
```

If balance is 0:
```bash
btcpc faucet claim myproject
# Or via Telegram: message @btcpcbot with /faucet
```

Faucet gives 10 BTCPC — covers ~100,000 average inference calls.

### Step 6 — Generate an API key

Check first:
```bash
grep BTCPC_API_KEY .btcpc/wallet.env 2>/dev/null
```

If `BTCPC_API_KEY` is empty or missing:
```bash
btcpc wallet api-key-gen --mnemonic "your twelve words here"
# Registers a random 256-bit key on-chain.
# Writes BTCPC_ACCOUNT and BTCPC_API_KEY to .btcpc/wallet.env
```

```bash
echo '.btcpc/wallet.env' >> .gitignore
```

### Step 7 — Set environment variables

```bash
BTCPC_ACCOUNT=myproject
BTCPC_API_KEY=<64-char hex from api-key-gen>
BTCPC_API_URL=https://btcpc.net   # override to use a local node
```

Load from `.btcpc/wallet.env`:
```bash
export $(grep -v '^#' .btcpc/wallet.env | xargs)
```

---

## Inference — `/v1/chat/completions`

**Auth required. Fee: 100 dreams per token (min 1,000 dreams per request).**

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

The response includes `usage.fee_dreams` showing exact cost.

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
| `402` | Insufficient balance | Claim faucet — see below |
| `429` | Rate limited (60 req/60s per IP) | Back off |
| `503` | No miner available for this model | Try a different model |

### On 402: auto-claim faucet (agent pattern)

```rust
match client.chat_completions(messages, None).await {
    Err(e) if e.to_string().contains("402") => {
        client.faucet_claim(&account).await?;
        client.chat_completions(messages, None).await?
    }
    r => r?,
}
```

---

## Balance

```bash
curl https://btcpc.net/api/balance/myproject
```

```rust
let bal = client.balance("myproject").await?;
println!("{} BTCPC ({} dreams)", bal.balance, bal.dreams);
```

---

## Git hosting — LinkGit

```bash
btcpc repo init myproject
git remote add btcpc https://git.btcpc.net/myproject/myrepo
git push btcpc main

git clone https://git.btcpc.net/myproject/myrepo
```

Via API:
```bash
curl -X POST https://btcpc.net/api/linkgit/repo/create \
  -H "Content-Type: application/json" \
  -d '{"owner": "myproject", "name": "myrepo", "private": false}'
```

---

## Storage heartbeat

Nodes that store and serve data earn from the storage pool each epoch:

```bash
curl -X POST https://btcpc.net/api/entry \
  -H "Content-Type: application/json" \
  -d '{
    "type": "STORAGE_HEARTBEAT",
    "node_id": "myproject",
    "bytes_stored": 1073741824,
    "bytes_proven": 1073741824,
    "proof_valid": true,
    "epoch": <current_epoch>
  }'
```

Get the current epoch:
```bash
curl https://btcpc.net/api/latest
```

---

## Inference marketplace (async, for long jobs)

For jobs over ~30 seconds, use the task marketplace:

```bash
btcpc inference post \
  --account myproject \
  --model llama3:70b \
  --input "Analyse the following 50k-word document..." \
  --max-fee 100000

btcpc inference jobs                  # list all jobs
btcpc inference job <job_id>          # get status + result
```

Job lifecycle: `Posted → Awarded → Completed → Verified → Paid`

---

## Available models

```bash
curl https://btcpc.net/api/node/models
```

Common models on testnet: `dolphin-llama3`, `llama3`, `mistral`, `qwen2.5`, `gemma2`.

---

## Checklist

- [ ] Check for existing wallet before creating a new one
- [ ] `btcpc wallet create --account <name>`
- [ ] `btcpc key generate`
- [ ] `btcpc key register --account <name> --role posting`
- [ ] `btcpc faucet claim <name>`
- [ ] `btcpc wallet api-key-gen --mnemonic "..."`
- [ ] `echo '.btcpc/wallet.env' >> .gitignore`
- [ ] Set `BTCPC_ACCOUNT`, `BTCPC_API_KEY`, `BTCPC_API_URL` in your env
- [ ] Handle `402` by claiming faucet and retrying once
- [ ] Never hardcode `BTCPC_API_URL` — always read from env

# BTCPC Agent Integration Guide

How any agent — Claude, an autonomous loop, a bot, a cron worker — requests
inference and agentic work from the BTCPC network deterministically and
seamlessly.

---

## Core Principle

BTCPC is a **sovereign AI inference network**. Agents never call Ollama or
OpenAI directly. All inference goes through BTCPC. The network:

- Routes jobs to miners running the appropriate model
- Pays miners from the requester's BTCPC balance
- Returns cryptographically-verified results
- Scales across every node on the network

---

## Setup (all languages)

### Environment variables

Every satellite project must have these set (via `.envbtcpc`, `.env`, or
system environment):

```bash
BTCPC_API_URL=https://btcpc.net      # canonical — never localhost
BTCPC_API_KEY=yourprojectname        # your BTCPC account name (optional, enables billing)
BTCPC_ACCOUNT=yourprojectname        # account on chain
BTCPC_MODEL=qwen3.5:27b              # default model (override per-call)
```

`BTCPC_API_KEY` is your BTCPC account name. The API is usable without it
(rate-limited to 60 req/min per IP), but setting it enables per-account
usage tracking and higher limits in future releases.

Never hardcode keys or URLs in source. The API URL fallback in code must
always be `https://btcpc.net`, never `localhost`.

### Verify the endpoint is working

```bash
# List available models
curl https://btcpc.net/v1/models

# Test a chat completion (no auth required)
curl -X POST https://btcpc.net/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"ping"}]}'
```

---

## Node.js / JavaScript

### Option A — SDK (recommended)

```js
const BTCPC = require('../btcpc/sdk');  // or: require('@btcpc/sdk')

const btcpc = new BTCPC({
  apiKey: process.env.BTCPC_API_KEY,
  account: process.env.BTCPC_ACCOUNT,
});

// Single question
const answer = await btcpc.ask({ prompt: 'Summarise this article: ...' });

// Chat with system prompt
const response = await btcpc.chat({
  system: 'You are a financial analyst. Be concise.',
  messages: [{ role: 'user', content: 'What is the risk in this portfolio?' }],
  model: 'qwen3.5:27b',     // optional override
  maxTokens: 512,
});
const text = response.choices[0].message.content;
```

### Option B — OpenAI-compatible (drop-in replacement)

```js
const axios = require('axios');

const BTCPC_API = process.env.BTCPC_API_URL || 'https://btcpc.net';

const res = await axios.post(`${BTCPC_API}/v1/chat/completions`, {
  model: process.env.BTCPC_MODEL || 'qwen3.5:27b',
  messages: [
    { role: 'system', content: 'You are a helpful assistant.' },
    { role: 'user', content: prompt }
  ],
  max_tokens: 1024,
}, { headers: { Authorization: `Bearer ${process.env.BTCPC_API_KEY}` } });

const text = res.data.choices[0].message.content;
```

### Option C — Async submit/poll (long jobs, background workers)

Use this when the job may take more than a few seconds (large model,
long context, batch processing).

```js
const axios = require('axios');

const BTCPC_API = process.env.BTCPC_API_URL || 'https://btcpc.net';
const AUTH = { headers: { Authorization: `Bearer ${process.env.BTCPC_API_KEY}` } };

async function btcpcInference(messages, opts = {}) {
  // 1. Submit
  const { data: sub } = await axios.post(`${BTCPC_API}/v1/inference/submit`, {
    model: opts.model || process.env.BTCPC_MODEL || 'qwen3.5:27b',
    messages,
    max_tokens: opts.maxTokens || 1024,
    temperature: opts.temperature || 0.7,
  }, AUTH);

  const jobId = sub.job_id;

  // 2. Poll (exponential backoff, max 10 min)
  let delay = 2000;
  for (let attempt = 0; attempt < 40; attempt++) {
    await new Promise(r => setTimeout(r, delay));
    delay = Math.min(delay * 1.5, 15000);

    const { data: job } = await axios.get(`${BTCPC_API}/v1/inference/${jobId}`, AUTH);

    if (job.status === 'completed') {
      return job.result?.choices?.[0]?.message?.content || job.result;
    }
    if (job.status === 'failed') {
      throw new Error(`BTCPC job ${jobId} failed: ${job.error || 'unknown'}`);
    }
    // status: 'pending' | 'running' — keep polling
  }
  throw new Error(`BTCPC job ${jobId} timed out`);
}
```

---

## Rust

Add to `Cargo.toml`:
```toml
[dependencies]
reqwest   = { version = "0.12", features = ["json"] }
serde_json = "1"
tokio     = { version = "1", features = ["time"] }
```

Copy `btcpc_client.rs` from any satellite repo (e.g. `BusWingSpread/btcpc_client.rs`)
or paste inline:

```rust
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::sleep;

fn api_url() -> String {
    std::env::var("BTCPC_API_URL").unwrap_or_else(|_| "https://btcpc.net".to_string())
}

/// Single-shot chat — fast path for interactive use.
pub async fn btcpc_ask(
    prompt: &str,
    system: Option<&str>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let base = api_url();
    let key  = std::env::var("BTCPC_API_KEY").unwrap_or_default();
    let model = std::env::var("BTCPC_MODEL").unwrap_or_else(|_| "qwen3.5:27b".to_string());

    let mut messages = vec![];
    if let Some(s) = system {
        messages.push(json!({"role": "system", "content": s}));
    }
    messages.push(json!({"role": "user", "content": prompt}));

    let res: Value = Client::new()
        .post(format!("{}/v1/chat/completions", base.trim_end_matches('/')))
        .bearer_auth(&key)
        .json(&json!({ "model": model, "messages": messages, "max_tokens": 1024 }))
        .send().await?.json().await?;

    Ok(res["choices"][0]["message"]["content"].as_str().unwrap_or("").to_string())
}

/// Async submit + poll — for long jobs or background workers.
pub async fn btcpc_infer(
    messages: Vec<Value>,
    model: Option<&str>,
    timeout_secs: u64,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let base  = api_url();
    let key   = std::env::var("BTCPC_API_KEY").unwrap_or_default();
    let model = model.unwrap_or("qwen3.5:27b");
    let client = Client::new();

    let sub: Value = client
        .post(format!("{}/v1/inference/submit", base.trim_end_matches('/')))
        .bearer_auth(&key)
        .json(&json!({ "model": model, "messages": messages, "max_tokens": 1024 }))
        .send().await?.json().await?;

    let job_id = sub["job_id"].as_str().ok_or("missing job_id")?.to_string();

    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    let mut delay = Duration::from_millis(2000);

    while std::time::Instant::now() < deadline {
        sleep(delay).await;
        delay = (delay.mul_f32(1.5)).min(Duration::from_secs(15));

        let poll: Value = client
            .get(format!("{}/v1/inference/{}", base.trim_end_matches('/'), job_id))
            .bearer_auth(&key)
            .send().await?.json().await?;

        match poll["status"].as_str() {
            Some("completed") => return Ok(
                poll["result"]["choices"][0]["message"]["content"]
                    .as_str().unwrap_or("").to_string()
            ),
            Some("failed") => return Err(
                format!("job {} failed: {}", job_id, poll["error"]).into()
            ),
            _ => {}
        }
    }

    Err(format!("job {} timed out after {}s", job_id, timeout_secs).into())
}
```

---

## Agentic work — multi-step tasks

For tasks that require multiple inference steps, tool calls, or decisions,
structure them as a loop that builds context across BTCPC calls. The agent
maintains state; BTCPC executes each inference step.

```js
// Agentic loop pattern
async function agentRun(goal, tools = [], maxSteps = 10) {
  const messages = [
    { role: 'system', content: AGENT_SYSTEM_PROMPT },
    { role: 'user', content: goal }
  ];

  for (let step = 0; step < maxSteps; step++) {
    const response = await btcpc.chat({ messages, model: 'qwen3.5:27b' });
    const assistantMsg = response.choices[0].message;
    messages.push(assistantMsg);

    // If the model signals completion, return
    if (assistantMsg.content.includes('TASK_COMPLETE')) {
      return extractResult(assistantMsg.content);
    }

    // If it wants to call a tool, execute it and feed results back
    const toolCall = parseToolCall(assistantMsg.content);
    if (toolCall) {
      const result = await executeTool(toolCall, tools);
      messages.push({ role: 'user', content: `Tool result: ${JSON.stringify(result)}` });
    }
  }

  throw new Error('Agent exceeded max steps without completing');
}
```

### System prompt for agentic mode

```
You are an autonomous agent running on the BTCPC network.
Complete the given task step by step.
When you need information, use the available tools by responding with:
  TOOL: <tool_name> ARGS: <json_args>
When the task is complete, respond with:
  TASK_COMPLETE
  RESULT: <final result>
Be concise. Think before acting. Verify results before marking complete.
```

---

## Model selection guide

| Task | Recommended model | Why |
|------|-------------------|-----|
| Short Q&A, classification | `qwen3.5:9b` | Fast, cheap, sufficient |
| General reasoning, summarisation | `qwen3.5:27b` | Good balance |
| Code generation | `deepseek-coder:33b` | Trained on code |
| Long context / large docs | `llama3.1:70b` | Larger context window |
| Math / logic | `qwen3.5:72b` | Best reasoning |
| Real-time / latency-sensitive | Any `:7b` or `:9b` | Smallest available |

Work value on BTCPC scales with verified parameter count — larger models
cost more per token but produce higher-value results for miners.

To see live model availability and pricing:
```js
const models = await btcpc.models();
const pricing = await btcpc.pricing('qwen3.5:27b');
```

---

## Error handling

```js
try {
  const result = await btcpc.ask({ prompt });
} catch (err) {
  if (err.status === 402) {
    // Insufficient balance — claim faucet and retry once
    await btcpc.faucetClaim();
    const result = await btcpc.ask({ prompt });
  } else if (err.status === 503 || err.message.includes('not reachable')) {
    // Network unavailable — queue for retry
    await queueForRetry(task);
  } else {
    throw err;
  }
}
```

Common status codes:
- `402` — insufficient BTCPC balance (faucet or top up)
- `429` — rate limited (back off)
- `503` — no miner available for this model (try a different model or retry)
- `504` — job timeout (break into smaller chunks)

---

## Seamless UX — what users see

Users should never see inference loading states longer than necessary.
Follow this pattern for user-facing features:

1. **Submit immediately** — fire the job on user action, return a job ID
2. **Optimistic UI** — show a spinner or progress indicator
3. **Stream if possible** — use `/v1/chat/completions` with `stream: true`
   for token-by-token output (same as OpenAI streaming)
4. **Poll in background** — for async jobs, poll and update the UI when ready
5. **Only surface errors users can act on** — "network busy, retrying..."
   rather than raw status codes

```js
// Streaming example
const res = await fetch(`${BTCPC_API}/v1/chat/completions`, {
  method: 'POST',
  headers: { Authorization: `Bearer ${KEY}`, 'Content-Type': 'application/json' },
  body: JSON.stringify({ model, messages, stream: true }),
});

const reader = res.body.getReader();
const decoder = new TextDecoder();
let buffer = '';

while (true) {
  const { done, value } = await reader.read();
  if (done) break;
  buffer += decoder.decode(value, { stream: true });
  const lines = buffer.split('\n');
  buffer = lines.pop();
  for (const line of lines) {
    if (!line.startsWith('data: ')) continue;
    const data = line.slice(6);
    if (data === '[DONE]') break;
    const chunk = JSON.parse(data);
    const token = chunk.choices?.[0]?.delta?.content || '';
    if (token) appendToUI(token);  // render each token as it arrives
  }
}
```

---

## Balance and project tracking

```js
// Check balance (no auth needed for public accounts)
const bal = await btcpc.balance('myproject');
console.log(bal.balance.BTCPC, 'BTCPC available');

// Get project spend and revenue
const project = await btcpc.project();
console.log(project);

// Set revenue split (e.g. 70% to you, 30% reinvested)
await btcpc.setRevenueSplit('myproject', [
  { account: 'shindevlin', percent: 70 },
  { account: 'myproject', percent: 30 },
]);
```

---

## Quick-start checklist for a new satellite project

**Rust:**
- [ ] Copy `BusWingSpread/btcpc_client.rs` into `src/btcpc.rs` (or inline the functions)
- [ ] Add `reqwest`, `serde_json`, `tokio` to `Cargo.toml`
- [ ] Create `.envbtcpc` with `BTCPC_API_URL=https://btcpc.net` + `BTCPC_API_KEY`
- [ ] Add `.envbtcpc` to `.gitignore`
- [ ] Default fallback in code: `unwrap_or_else(|_| "https://btcpc.net".to_string())` — never localhost
- [ ] Run `scripts/wire-satellites.sh` from the btcpc repo to verify wiring

**Node.js (bots/scripts):**
- [ ] Copy `sdk/btcpc-client.js` into the repo (or `require('../btcpc/sdk')`)
- [ ] Create `.envbtcpc` (or add to `.env`) with `BTCPC_API_URL=https://btcpc.net` + `BTCPC_API_KEY`
- [ ] Add env file to `.gitignore`
- [ ] Fallback in code: `process.env.BTCPC_API_URL || 'https://btcpc.net'` — never localhost
- [ ] Run `scripts/wire-satellites.sh` from the btcpc repo to verify wiring

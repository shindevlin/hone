# HONE Inference Integration — Guide for Consuming Projects

_As of 2026-07-05. Audience: any external project that integrates with a HONE node for
LLM inference (bots, services, workers, marketplaces). Read this before building against
older assumptions._

---

## ⚠️ Naming first — the chain was renamed (HONE → HONE)

**This chain USED TO BE CALLED "HONE". It is NOW "HONE".** If your project has any
existing integration, config, or documentation referencing HONE, that was **this same
chain**. The rename (2026-07-05) is:

| Was (HONE) | Now (HONE) |
|---|---|
| Brand / chain / network / token name: **HONE** | **HONE** |
| Base unit (smallest denomination): **dream** | **hunit** (1 HONE = 10,000,000,000 hunits) |
| chain_id: **hone-2** | **`hone`** (mainnet), **`hone-testnet`** (testnet) |
| Rust crates: **hone-\*** | **hone-\*** (e.g. `hone-node` → `hone-node`) |
| Env vars: **HONE_\*** | **HONE_\*** |
| Domain | **honemesh.net** (this is ONLY the domain — the brand is **HONE**) |

Anywhere you still see `hone` / `HONE` / `dream`, treat it as a **not-yet-migrated
tail, not the current name**. Update your references to the HONE names.

HONE is a Rust "Proof of Compute" sovereign chain; the node lives in `rust/hone-node/`.
This document covers **how it runs LLM inference now**.

---

## What changed — Ollama daemon → embedded candle GGUF (in-process)

**OLD:** the node called an external **Ollama daemon** over HTTP
(`{OLLAMA_URL}/api/chat`) from ~6 scattered call sites. When that daemon was down,
inference `503`'d — this is literally what took down the `bullship` service at launch.
An external daemon means a single point of failure, an extra port, and an extra process
to run and monitor.

**NEW:** the node runs a **candle GGUF model IN-PROCESS**
([huggingface/candle](https://github.com/huggingface/candle)). If the node is alive,
inference is alive — no daemon, no separate port, no `503`. Ported from the proven mobile
stack (`hone-android/src/llm.rs`).

---

## The canonical surface (build against this)

One module owns all inference: **`rust/hone-node/src/inference_engine.rs`**.

Public API:

- `inference_engine::chat(ChatRequest) -> Result<ChatResponse>`
- `inference_engine::available() -> bool`
- `inference_engine::warm_up()`

Request shape (OpenAI/Ollama-compatible, so payloads are familiar):

```rust
ChatRequest {
    model: String,              // advisory for the embedded backend; forwarded verbatim to an external one
    messages: Vec<Message>,     // Message { role: String, content: String }
    max_tokens: usize,
}
```

`model` is **advisory** for the embedded backend (it serves the single loaded GGUF); it
is forwarded verbatim to an external backend.

---

## Backend selection (decided once, at node startup)

In priority order:

1. **`INFERENCE_URL`** set (or the legacy **`OLLAMA_URL`** alias) → route to that external
   OpenAI/Ollama-compatible server. **This is how a GPU node points at vLLM.**
2. else, node built with cargo feature **`inference-embedded`** (ON by default) →
   **embedded candle GGUF**, in-process.
3. else → inference **unavailable** (relay-only node).

---

## Embedded model details

- **Default model:** `Qwen/Qwen2.5-0.5B-Instruct-GGUF`, file
  `qwen2.5-0.5b-instruct-q4_k_m.gguf`; tokenizer from `Qwen/Qwen2.5-0.5B-Instruct`.
- Fetched via `hf-hub` on first use (`ensure_ready()`), cached under a model dir that is
  overridable by **`HONE_MODEL_DIR`**.
- The `inference-embedded` cargo feature pulls `candle-core` / `-transformers` / `-nn` +
  `tokenizers` + `hf-hub`. Turn it off (`--no-default-features`) for a lean relay/proxy
  node that only points at an external `INFERENCE_URL`.

---

## Integration guidance for your project

- **Do NOT assume an Ollama daemon exists** or that a port like `11434` is listening.
  Default HONE nodes serve inference internally with **no exposed inference port**.
- If you need to point a HONE node at **your** inference server (vLLM, an Ollama box, any
  OpenAI-compatible endpoint), set **`INFERENCE_URL`** on the node. `OLLAMA_URL` still
  works as a deprecated alias, but prefer `INFERENCE_URL` in new config.
- If you consume inference via the node's **HTTP API** rather than the Rust module, keep
  using the node's existing API routes — those now resolve through `inference_engine`
  internally.

---

## Caveat — migration not 100% swept

The canonical path is `inference_engine::chat`, but a handful of **legacy call sites**
in the node (parts of `api.rs`, `main.rs`, `agent_session.rs`, `hardware_probe.rs`) still
read `OLLAMA_URL` directly. So `OLLAMA_URL` remains meaningful as a fallback today. Treat
`inference_engine` as the **intended single source of truth**, and expect the remaining
`OLLAMA_URL` reads to be consolidated behind it over time.

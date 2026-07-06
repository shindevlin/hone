---
title: BTCPC Embedded Inference — implementation spec (non-fragile, embedded candle)
description: One inference module, embedded candle GGUF engine, no external daemon — so a node can never 503 because "the model server is down"
author: Shin Devlin
status: spec / ready to build
supersedes: reliance on external Ollama daemon
see_also: INFERENCE_MIGRATION_NOTES.md (running decision log), INFERENCE_ENGINE_ANALYSIS.md
---

# Embedded Inference — Implementation Spec

**Directive (Shin):** make it not fragile, and make it make the most sense.
Then: "do candle, and keep copious notes." (see INFERENCE_MIGRATION_NOTES.md)

**The decision:** the inference engine runs **inside the node process** (embedded
**candle** GGUF — pure Rust), not as a separate server. If the node is alive,
inference is alive. This permanently kills the failure we hit at launch (a node
returning 503 because its Ollama daemon wasn't running). All inference calls go
through **one module** instead of being scattered — removing the second, quieter
fragility.

**Why candle, not llama.cpp:** the node already links C/C++ (rocksdb, openssl),
so "avoid FFI" was a false premise — but candle wins anyway because **we already
run it**: `rust/btcpc-android/src/llm.rs` is a complete, working candle GGUF impl
(Qwen2.5-0.5B). Using candle in the node means ONE Rust inference engine across
phone + node. The migration is largely a PORT of llm.rs, not new code. Full
reasoning in INFERENCE_ENGINE_ANALYSIS.md §3 and the notes doc §2.

## 1. Why this is the non-fragile choice

Two sources of fragility today, both fixed here:

1. **External daemon.** The node calls `http://localhost:11434` (Ollama). If that
   process isn't up / has no model → 503. Fix: **embed the engine** — no separate
   process to fail. Inference is a function call, not a network hop.
2. **Scattered call sites.** `OLLAMA_URL` + `http.post("/api/chat")` is duplicated
   in ~6 places (`agent_session.rs`, `agent_worker.rs`, `api.rs` ×4). Each has its
   own error handling and can drift. Fix: **one `inference` module** every caller
   uses; the backend choice lives in exactly one place.

## 2. The single inference interface

Create `rust/hone-node/src/inference.rs` — the ONE entry point:

```rust
pub struct ChatRequest { pub model: String, pub messages: Vec<Message>, pub max_tokens: Option<u32>, pub stream: bool }
pub struct ChatResponse { pub content: String, /* + usage */ }

/// Non-streaming completion. Returns the assistant content.
pub async fn chat(req: ChatRequest) -> anyhow::Result<ChatResponse>;

/// Streaming completion — yields tokens as they generate (for /v1/chat/completions SSE).
pub fn chat_stream(req: ChatRequest) -> impl Stream<Item = Result<Token, Error>>;

/// Is inference available right now? (used by node/info has_ollama → has_inference)
pub fn available() -> bool;   // embedded: true once a model is loaded — never a network probe
```

Every existing caller (agent_session, agent_worker, the 4 api.rs sites) is
rewritten to call `inference::chat` / `chat_stream`. They stop knowing about
Ollama, URLs, or HTTP. The response shape they already parse
(`message.content` non-streaming; streamed chunks for SSE) is preserved by the
module, so callers change minimally.

## 3. The backend behind that interface

`inference.rs` selects a backend once, at startup, in priority order:

1. **Embedded candle GGUF (default, the non-fragile path).** Pure-Rust candle
   loads a GGUF model in-process at boot (port of `btcpc-android/src/llm.rs`:
   `candle_transformers::models::quantized_llama::ModelWeights::from_gguf`) and
   serves `chat`/`chat_stream` as direct function calls. **No daemon, no port,
   no 503, no C++ FFI.**
2. **External HTTP (opt-in escape hatch).** If `INFERENCE_URL` is set, route to an
   external OpenAI/Ollama-compatible server instead (this is how a GPU node points
   at **vLLM** for the throughput tier). Keeps the pluggability without making it
   the default.
3. **`OLLAMA_URL`** stays as a deprecated alias for `INFERENCE_URL` (back-compat)
   — existing deployments keep working, but it's no longer the recommended path.

```
inference::chat(req)
  └─ backend = OnceCell<Backend>   // chosen once at startup
       ├─ Backend::Embedded(candle GGUF ModelWeights)  ← default: in-process, always alive, pure Rust
       └─ Backend::Http(url)                           ← if INFERENCE_URL / OLLAMA_URL set (e.g. vLLM GPU node)
```

Rationale for embedded-default over pure-Rust-node-calls-external-server:
the external-server option is simpler to *write*, but it **reintroduces the exact
fragility we're removing** (there must be a server up somewhere). Shin's directive
is "not fragile" → embedded wins. And candle makes embedded *cheap* because we
already have the working impl in btcpc-android — the node just reuses it. One
Rust engine, phone + node.

## 4. Model management (also non-fragile)

- The embedded backend loads a **GGUF** file from a configured path
  (`HONE_MODEL_PATH`), default a small model to match the phone tier
  (`qwen2.5-0.5b` GGUF). Small = fast cold start, runs on CPU/ARM (the Nebra).
- On boot, if the model file is missing, the node **downloads it once** from a
  content-addressed store (BTCPC-FS or a pinned URL) and caches it. Missing model
  is a one-time fetch, not a per-request failure.
- `inference::available()` returns true once the model is resident — the
  node/info `has_ollama` field becomes `has_inference` and reflects real
  readiness, not a network probe that can flap.

## 5. Tiers (how this serves both usage profiles)

| Node type | Backend | Config |
|---|---|---|
| Ordinary device (phone-adjacent, Nebra, laptop) | **Embedded candle GGUF** (default) | nothing — just works |
| GPU revenue node (high throughput) | **vLLM** external | `INFERENCE_URL=http://<vllm-host>:8000` |
| Phone micronode | **candle GGUF** (btcpc-android/src/llm.rs — already candle) | already proven; the node reuses this |

One node type never has to think about inference infra; the heavy node opts into
vLLM with one env var. Ollama is dropped as a concept.

## 6. Build plan (incremental, each step ships independently)

1. **Consolidate first (no behavior change):** create `inference.rs` with the
   interface in §2, backed *initially* by the existing HTTP-to-Ollama logic.
   Rewrite all ~6 call sites to use it. Now there's ONE inference path. Ship.
   *(This alone removes the scattered-duplication fragility and is low-risk.)*
2. **Add the embedded backend** behind a cargo feature `inference-embedded`
   (candle, porting `btcpc-android/src/llm.rs`). `inference.rs` picks Embedded when
   built with the feature and no `INFERENCE_URL` is set. Verify parity: existing
   callers get identical output shape.
3. **Model bootstrap:** GGUF path + one-time download + `available()` wired to
   model residency.
4. **Rename env:** `INFERENCE_URL` primary, `OLLAMA_URL` aliased. Update
   node/info `has_ollama` → `has_inference`.
5. **Make embedded the default build.** Ollama becomes "external HTTP, if you
   insist" — documented as deprecated.
6. **Document the vLLM tier** for GPU operators.

## 7. Acceptance (how we know it's non-fragile)

- Start a node with **no Ollama installed, no INFERENCE_URL, no network** →
  `POST /v1/chat/completions` returns a real completion (embedded model). This is
  the exact scenario that 503'd at launch; it must now pass.
- Kill any external server → node inference is unaffected (nothing external to
  kill).
- All existing callers (agent tasks, sessions, the marketplace endpoint) work
  unchanged.

## 8. Decision locked (from Shin)
- **Embedded, not fragile.** Engine runs in-process; no external daemon in the
  default path.
- **Embedded candle GGUF** (pure Rust) — reuse the proven
  `btcpc-android/src/llm.rs` impl; one inference engine across phone + node.
- **vLLM stays available** as the opt-in GPU throughput tier via `INFERENCE_URL`.
- **Ollama is dropped** as the default/recommended path (kept only as an HTTP
  alias for back-compat).

## 9. Open watch-items (candle-specific — see notes doc §8)
1. **rustc 1.90 pin:** the node is pinned to 1.90.0 (ICE). Verify candle-core 0.10
   + deps compile on 1.90 before committing. (btcpc-android uses candle 0.10 —
   check its toolchain.)
2. **aarch64 Nebra cross-compile:** confirm candle's CPU backend cross-compiles
   clean for `aarch64-linux-gnu` (pure Rust should help vs a C++ engine).
3. **Streaming:** llm.rs returns a full String; the node's /v1/chat/completions is
   SSE-streaming. Add a streaming generate variant that yields tokens to the SSE
   sender.
4. **Build weight:** candle is heavy — gate behind the `inference-embedded` cargo
   feature so non-inference nodes stay lean; background the one-time ~400 MB GGUF
   download so it doesn't block startup.

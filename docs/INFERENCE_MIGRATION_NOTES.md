---
title: BTCPC Inference Migration — running notes & decision log
description: Copious working notes for the Ollama → embedded-candle migration. Append-only; newest context near the bottom of each section.
author: Shin Devlin
status: living document
---

# Inference Migration — Notes & Decision Log

> **Purpose:** a copious, append-only record of the reasoning, decisions,
> findings, and gotchas for moving BTCPC inference off Ollama to embedded candle.
> Anyone (human or agent) picking this up should be able to read this and know
> exactly why every choice was made and where the bodies are buried.

---

## 0. The decision, in one line
Move node inference from **external Ollama daemon** → **embedded candle GGUF,
in-process**, unifying phone + node on ONE Rust inference engine. vLLM stays as
an opt-in external tier for GPU nodes. Ollama and tract-onnx are both retired.

## 1. Why we're doing this at all (context)
- **Launch-day trigger (2026-07-04):** bullship's inference returned HTTP 503 —
  "inference unavailable: error sending request to http://localhost:11434". The
  node's Ollama daemon wasn't running / had no model. The chain-side path (auth,
  billing, key) all worked; the *engine* was the single point of failure.
- Shin: "make it not fragile, and make it make the most sense." Then: "do candle,
  and keep copious notes."
- **Fragility = an external process that can be down.** The fix is to put
  inference *inside* the node, so it's alive whenever the node is.

## 2. Why candle (pure Rust) over llama.cpp (C++ FFI) — the real analysis
The initial instinct was llama.cpp (Ollama is built on it). Shin pushed: "why C
vs pure Rust?" Investigation on our own machine settled it:

- **The node is ALREADY not pure Rust.** It links C/C++ via FFI heavily:
  `rocksdb` (the entire chain state store is a C++ database), vendored `openssl`,
  `ring`. So "avoid C++ FFI to stay pure Rust" was a FALSE premise — rocksdb is
  more central than inference would ever be.
- **We ALREADY run candle for GGUF.** `rust/btcpc-android/Cargo.toml` uses
  `candle-core`/`candle-transformers`/`candle-nn`, and
  `rust/btcpc-android/src/llm.rs` is a COMPLETE working candle GGUF inference
  impl (loads Qwen2.5-0.5B GGUF, generates tokens). It's proven in-house.
- **Decisive reason for candle:** ONE engine across phone + node. Same candle
  GGUF path everywhere instead of tract-onnx (phone) + llama.cpp (node) +
  Ollama (legacy). Consistency = "makes most sense."
- **The throughput ceiling doesn't bite:** candle < llama.cpp on raw CPU/GPU
  throughput, but the breadth tier (phones, Nebra) does one request at a time.
  The GPU throughput tier uses **vLLM external** anyway — candle was never for
  that. So candle's weakness lands where it doesn't matter.

**Conclusion:** candle-embedded for the default/breadth tier; vLLM external for
GPU; drop Ollama + tract-onnx. No C++ needed for inference (candle is Rust),
even though the node already has C++ deps.

## 3. The gift: btcpc-android/src/llm.rs already does this
The hard part is written. `rust/btcpc-android/src/llm.rs` (verified 2026-07-04):
- `candle_transformers::models::quantized_llama::ModelWeights::from_gguf(...)`
  — loads GGUF weights, pure Rust.
- Model: `Qwen/Qwen2.5-0.5B-Instruct-GGUF`, file
  `qwen2.5-0.5b-instruct-q4_k_m.gguf` (~400 MB), tokenizer from
  `Qwen/Qwen2.5-0.5B-Instruct`.
- Generation loop: `model.forward(&input, pos)?` → logits → sample → append.
- Auto-downloads the GGUF from HF if missing; caches to a model dir.

**The node migration = port this into btcpc-node behind the unified inference
module.** Do NOT reinvent it — lift the proven pattern.

## 4. Where the node calls inference today (the seams to replace)
All call `POST {OLLAMA_URL}/api/chat` or `/api/generate` over HTTP, each doing
its own `std::env::var("OLLAMA_URL")`:
- `rust/hone-node/src/agent_session.rs:146,175` — chat; parses
  `response["message"]["content"]`.
- `rust/hone-node/src/agent_worker.rs:292,302,321` — /api/generate + /api/chat.
- `rust/hone-node/src/api.rs:1436` (has_ollama / node info),
  `:3752,3775` (a chat call), `:7670,7697,7754,7798` (the
  `/v1/chat/completions` STREAMING path — SSE, parses streamed chunks).
- `api.rs:1300` — `has_ollama` capability flag → becomes `has_inference`.

Two response shapes to preserve: **non-streaming** (`message.content`) and
**streaming** (SSE token chunks for /v1/chat/completions).

## 5. The design (unified module + embedded backend)
- New `rust/hone-node/src/inference.rs` — THE single entry point:
  `chat()`, `chat_stream()`, `available()`. Every caller in §4 uses it; none know
  about Ollama/URLs anymore. (Consolidation alone removes the scattered-dup
  fragility — do it FIRST, no behavior change.)
- Backend chosen once at startup:
  1. **Embedded candle GGUF** (default) — port of llm.rs, in-process, always alive.
  2. **External HTTP** if `INFERENCE_URL` set — for vLLM GPU tier.
  3. `OLLAMA_URL` = deprecated alias for `INFERENCE_URL`.
- Model: default `qwen2.5-0.5b-instruct-q4_k_m.gguf` (match the phone). One-time
  download + cache. `available()` = model resident (not a network probe).

## 6. Build order (each step ships independently)
1. **Consolidate (no behavior change):** create inference.rs wrapping the CURRENT
   Ollama-HTTP logic; rewrite all §4 call sites to use it. Ship. [removes
   scattered fragility, low risk]
2. **Add embedded candle backend** behind cargo feature `inference-embedded`,
   porting btcpc-android/src/llm.rs. inference.rs picks Embedded when built with
   the feature and no INFERENCE_URL.
3. **Model bootstrap:** GGUF path + one-time HF download + available() wiring.
4. **Rename env** OLLAMA_URL → INFERENCE_URL (alias kept); node/info has_ollama
   → has_inference.
5. **Embedded = default build.** Ollama demoted to "external HTTP if you insist."
6. **Doc the vLLM GPU tier.**

## 7. Acceptance (proves non-fragile)
Start a node with NO Ollama, NO INFERENCE_URL, NO network reachable →
`POST /v1/chat/completions` returns a real completion from the embedded model.
This is the EXACT scenario that 503'd at launch. Must pass.

## 8. Gotchas / watch-items (append as found)
- **rustc pin:** the node is pinned to rustc 1.90.0 (rust-toolchain.toml) due to
  an ICE. candle + its deps must build on 1.90. VERIFY candle-core 0.10 compiles
  on 1.90 before committing to it. (btcpc-android uses candle 0.10 — check what
  toolchain IT builds with.)
- **aarch64 Nebra:** candle must cross-compile for aarch64-linux-gnu (the Nebra).
  Pure Rust helps here vs llama.cpp's C++ build, but verify the candle CPU
  backend cross-compiles clean.
- **Streaming:** the /v1/chat/completions path is SSE-streaming. The embedded
  candle generate loop yields tokens one at a time — wire it to the SSE sender so
  streaming still works (llm.rs currently returns a full String; needs a
  streaming variant for the node).
- **Model size on nodes:** ~400 MB GGUF per node. Fine for laptops/Nebra; ensure
  the one-time download doesn't block node startup (background it, serve
  "warming up" until ready).
- **Binary size / build time:** candle pulls in a lot; the node build will grow.
  Gate behind the `inference-embedded` feature so non-inference nodes stay lean.

## 8b. Phone has TWO inference paths today — reconcile (found 2026-07-04)
There are currently two Android inference stacks (this is itself cruft to clean):
- `rust/btcpc-android/src/llm.rs` — **candle** GGUF (Qwen2.5-0.5B). This is the
  one to standardize on and port to the node.
- `android/rust/btcpc-miner/src/miner.rs` — **tract-onnx** (loads .onnx). Older
  path used by the Java app's libbtcpc_miner.so.
Standardizing the node on candle means the whole ecosystem converges on candle
GGUF; the tract-onnx path in btcpc-miner should eventually be retired/migrated to
candle too, so there's ONE engine everywhere. Not blocking the node migration,
but note it — don't build new things on tract-onnx.

## 9. Status log (append-only, dated)
- **2026-07-04:** Decision made (candle-embedded). Analysis docs written
  (INFERENCE_ENGINE_ANALYSIS.md, INFERENCE_EMBED_SPEC.md — revised to candle).
  Found btcpc-android/src/llm.rs as the proven port source. bullship inference
  currently works by pointing at Grouchly's node (Ollama+models) as a stopgap.
- **2026-07-04 (build started):** Shin said "just do candle, no wasted cycles" —
  building directly, not the phased hedge.
  - **NAME COLLISION CAUGHT:** `rust/hone-node/src/inference.rs` ALREADY EXISTS
    and is the inference-JOB-MARKETPLACE state machine (bid/award/verify/dispute,
    80KB) — NOT the model runner. Do NOT touch it. The model engine went into a
    NEW file `inference_engine.rs`. `inference.rs` = marketplace,
    `inference_engine.rs` = model execution. Keep them separate.
  - Added candle-core/transformers/nn 0.10 + tokenizers 0.20 + hf-hub 0.3 to
    btcpc-node/Cargo.toml (same versions as btcpc-android, which builds), all
    `optional=true` behind feature `inference-embedded` (default ON).
  - Wrote `inference_engine.rs`: unified `chat()`/`available()`/`warm_up()`,
    Backend = Http(vLLM/Ollama) | Embedded(candle) | None, selected once at
    startup. Embedded ports llm.rs's GGUF load + greedy generate. Uses
    std::sync::OnceLock (not once_cell — not a node dep).
  - `mod inference_engine;` registered in main.rs.
  - **✓ CANDLE COMPILES ON rustc 1.90** (the #1 risk — CLEARED). `cargo +1.90.0
    check --features inference-embedded`: candle-core 0.10.2, candle-nn,
    candle-transformers all built clean on the pinned toolchain. No ICE, no
    version conflict. The candle-vs-llama.cpp decision is validated. (One trivial
    borrow bug in my own inference_engine.rs — two get_vocab() temporaries — fixed
    by binding vocab once.)
  - **NEXT:** wire the ~6 old Ollama call sites (§4) to inference_engine::chat,
    call warm_up() at boot, rename has_ollama→has_inference. Streaming
    (/v1/chat/completions SSE) still needs a streaming generate variant — current
    port is non-streaming only. Then commit + push.

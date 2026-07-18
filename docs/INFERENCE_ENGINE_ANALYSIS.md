---
title: HONE Inference Engine — Ollama replacement analysis (llama.cpp vs vLLM, + Rust fork)
description: What to run inference on, grounded in how the node actually consumes it, and whether a Rust-native fork is worth it
author: Shin Devlin
status: analysis / recommendation
---

# Inference Engine Analysis

**Question (Shin):** Analyze what's best for HONE's usage — llama.cpp vs vLLM —
and whether we can fork one to be Rust-native.

**TL;DR recommendation:** Two tiers. **llama.cpp for the many ordinary-device
nodes** (phones, Nebra, laptops — the Proof-of-Compute breadth), **vLLM for the
few GPU nodes serving paid inference at scale.** On the fork question: **don't
fork either engine to Rust — bind to llama.cpp instead** (via `llama-cpp-2` /
FFI), embedded in the node. Forking a C++/Python inference engine to pure Rust is
a multi-month rewrite that loses upstream velocity for no real gain, because the
node already talks to inference over a thin HTTP boundary we control.

---

## 1. What HONE actually needs (grounded in the code)

The node's inference integration is **already abstracted behind a thin HTTP
boundary** — this is the key fact that shapes everything:

- `agent_session.rs`, `agent_worker.rs`, `api.rs` call `POST {OLLAMA_URL}/api/chat`
  and `/api/generate`, configurable via the `OLLAMA_URL` env var.
- Workloads: `/v1/chat/completions` (the paid inference marketplace), agent
  tasks (`/api/task/*`), and inference verification.

So the "engine" is a **replaceable HTTP backend**, not a deep in-process
dependency. Any replacement that serves an OpenAI-compatible or Ollama-shaped
HTTP API drops in by changing one env var. **This makes the choice about
operational fit, not a rewrite.**

HONE's inference has **two distinct usage profiles**:

| Profile | Who | Hardware | Load | Priority |
|---|---|---|---|---|
| **A — breadth (PoC)** | phones, Nebra, laptops, ordinary rigs | CPU / ARM / small GPU | 1 request at a time, small models | "even your phone mines" — the thesis |
| **B — throughput (revenue)** | dedicated GPU nodes | CUDA GPU, big VRAM | many concurrent, larger models | the paid `/v1/chat/completions` market |

Ollama serves **neither well**: it's a heavy external daemon (a wrapper around
llama.cpp) that adds a moving part and no batching. The gap we just hit at launch
— a node whose Ollama wasn't running returned 503 — is exactly this friction.

## 2. The engines against HONE's needs

### llama.cpp — the breadth tier (Profile A)
- **Runs on everything**: CPU, Apple Silicon, ARM (the Nebra!), consumer GPUs,
  even a Pi. GGUF quantized models — tiny footprint, fast cold start.
- **Embeddable**: it's a C++ library. You can link it *into* the node process
  (no separate daemon) via a Rust binding — kills the "Ollama not running" class
  of failure entirely.
- **This is the Proof-of-Compute enabler**: the reason ordinary devices can be
  earning inference nodes at all. Ollama already *is* llama.cpp underneath, so
  moving to it directly is the same models, minus the daemon.
- Weakness: modest throughput per node (no PagedAttention/continuous batching).
  Fine for Profile A — those nodes serve one request at a time anyway.

### vLLM — the throughput tier (Profile B)
- **Datacenter-grade**: PagedAttention + continuous batching = high concurrent
  throughput, the right tool for a node monetizing serious inference load.
- **Needs a real CUDA GPU + VRAM**. Won't run on ARM/CPU meaningfully — so it is
  **not** a breadth-tier option.
- Python/CUDA stack — heavier to operate, but that's acceptable for the few
  operators running paid GPU nodes.
- Serves an OpenAI-compatible API → drops behind `OLLAMA_URL` (rename to
  `INFERENCE_URL`) with no node code change.

### Verdict on the split
They are **not either/or — they are the two tiers.** A node operator picks the
engine that fits their hardware; the node doesn't care because both speak HTTP.
- Ordinary device → **llama.cpp** (ideally embedded, no daemon)
- GPU revenue node → **vLLM**
- **Drop Ollama** — it's the worst-of-both middle.

## 3. The fork-to-Rust question (the real ask)

**Should we fork llama.cpp or vLLM into pure Rust?** Short answer: **no — bind,
don't fork.** Reasoning:

1. **The node already isolates inference behind HTTP.** A pure-Rust engine buys
   us nothing at the integration layer we don't already have. The value of "Rust
   native" would be *embedding* (no external process), and we can get that
   without a rewrite (see below).
2. **Forking is a losing maintenance bet.** llama.cpp ships new quant formats,
   kernels, and model support weekly; vLLM the same for GPU. A Rust fork
   immediately falls behind upstream and we inherit a full ML-kernel maintenance
   burden — for a chain team, that's a permanent tax with no differentiation.
3. **Pure-Rust inference already exists and is the wrong fit for the heavy tier.**
   `candle` (HuggingFace, Rust) and `tract-onnx` (already used by the phone
   micronode!) are real Rust engines — but they trail llama.cpp/vLLM on model
   coverage, quantization, and GPU throughput. Good for the phone's tiny ONNX
   models; not for the marketplace.

### What to do instead — **embed llama.cpp via a Rust binding**
- Use **`llama-cpp-2`** (maintained Rust bindings to llama.cpp) or a thin FFI
  wrapper. This gives us the "Rust-native, no external daemon" property Shin
  wants — the node loads a GGUF model *in-process* and serves it — **without
  forking or maintaining kernels.** We ride llama.cpp's upstream velocity.
- Keep the **`OLLAMA_URL`/`INFERENCE_URL` HTTP path** as the pluggable escape
  hatch so a node *can* point at an external vLLM/llama.cpp server when it wants
  the throughput tier.
- The phone keeps **`tract-onnx`** (already in `android/rust/hone-miner`) — it's
  the right minimal engine for a phone; no change needed there.

## 4. Recommended architecture

```
                       ┌─────────────────────────────────────────┐
  node inference call  │  HONE node                              │
  (agent_worker/api)   │   ├─ EMBEDDED llama.cpp (llama-cpp-2)  ◄─┼─ Profile A: default,
                       │   │    in-process GGUF, no daemon        │   ordinary devices
                       │   └─ HTTP INFERENCE_URL (pluggable)    ◄─┼─ Profile B: point at
                       └─────────────────────────────────────────┘   an external vLLM GPU node
        phone micronode: tract-onnx in-process (unchanged)
```

- **Default (every node):** embedded llama.cpp — no Ollama, no separate process,
  works on CPU/ARM. Fixes the launch-day 503 permanently.
- **GPU revenue nodes:** run vLLM (or a big llama.cpp server) and set
  `INFERENCE_URL`; the node routes to it.
- **Rename `OLLAMA_URL` → `INFERENCE_URL`** (keep the old name as an alias for
  back-compat) so the config no longer implies a specific engine.

## 5. Migration plan (incremental, low-risk)

1. **Rename/alias the env:** `INFERENCE_URL` (falls back to `OLLAMA_URL`). Zero
   behavior change; decouples config from "Ollama".
2. **Add embedded llama.cpp** behind a cargo feature (`llama-embedded`) using
   `llama-cpp-2`: the node loads a GGUF and serves `/api/chat`-shaped calls
   in-process when no `INFERENCE_URL` is set. This is the big win — no daemon.
3. **Verify parity:** the node's existing `/api/chat` + `/api/generate` callers
   work unchanged against the embedded backend.
4. **Document the GPU tier:** how to run vLLM and point `INFERENCE_URL` at it.
5. **Deprecate Ollama** in docs — it still works via HTTP if someone runs it, but
   it's no longer the default or recommended path.

## 6. Open questions for Shin
1. Confirm the two-tier split (llama.cpp embedded default + vLLM for GPU nodes),
   vs. standardizing on one.
2. Embedded llama.cpp via `llama-cpp-2` FFI is the recommendation — OK to depend
   on a C++ library linked into the node, or do you want the node to stay pure
   Rust and always call inference over HTTP (external llama.cpp/vLLM server)?
   (Pure-Rust-node + external server is simpler to build but reintroduces the
   "is the server up?" operational risk. Embedded removes it but adds an FFI dep.)
3. GGUF model default for the breadth tier (e.g. qwen2.5-0.5b-GGUF to match the
   phone's model choice)?
```

# HONE Local-Model Survey — 2026-07-09

> **SUPERSEDED / narrow.** This file was a first pass that (a) wrongly proposed a
> "default" model and (b) assumed bit-identical cross-fleet output as the
> verification model. Both are wrong for a SOVEREIGN AI CHAIN: there is no default
> (operators choose), and output is NOT identical — verification is *reasonableness
> review*, not re-execution. See the fuller report from the
> `hone-sovereign-multimodal-ai` workflow (2026-07-09) for the corrected thesis:
> multi-modal catalogue + "objectively reasonable, not right" verification design.
> Kept below only for the candle-loadability facts, which remain accurate.

Companion to `scan-2026-07-06.md`. That scan covered inference *infrastructure*
(candle dispatch registry, batch-invariance, OPML fraud proofs). This one answers
a narrower question: **which local models candle can load today** — the
size/arch facts, not a recommendation of what to run (the chain picks nothing).

## The constraints that decide everything (before quality matters)

1. **candle quantized-loader arch support.** `inference_engine.rs` dispatches on
   `general.architecture` and today handles exactly `llama`, `qwen2`, `qwen3`.
   Anything else (`gemma3`, `phi3`, `smollm`→llama-ok, `stablelm`) needs a new
   `LoadedModel` variant + load arm + forward arm. candle-transformers *ships*
   `quantized_qwen2/qwen3/qwen2_moe/qwen3_moe` and `quantized_gemma3` etc., so
   most additions are wiring, not research — but each is a code change, not a
   drop-in GGUF.
2. **Phone CPU, greedy/argmax, deterministic if it touches mining.** CPU is the
   deliberate default (mining-verification determinism). Cross-fleet bit-identity
   is NOT guaranteed today (fp non-associativity, esp. BF16). Any model on the
   *verification* path inherits the open batch-invariance problem.
3. **No auto-download; operator picks from the central store.** "Interesting"
   also means small enough to distribute + pin (Q4_K_M, <1 GB ideal).

## Verdict table

| Model | Size (Q4) | candle arch | Fits HONE? | Why it's interesting |
|-------|-----------|-------------|------------|----------------------|
| **Qwen3-0.6B** | ~0.5 GB | `qwen3` ✅ (already wired) | **Best default** | Dual-mode thinking switch, 119 langs, runs in ~1 GB. Zero code change. |
| **Qwen3-1.7B** | ~1.4 GB | `qwen3` ✅ | **Best "smart" tier** | Meaningfully better reasoning, still phone-viable on 4 GB. Zero code change. |
| **SmolLM2-1.7B** | ~1.1 GB | `llama` ✅ | Yes, no change | Fastest tok/s in class (llama-arch → loads today). Good throughput miner model. |
| **Llama 3.2 1B/3B** | 0.8/2.2 GB | `llama` ✅ | Yes, no change | Proven, loads today. The safe fleet baseline. |
| **Gemma 3 270M** | ~0.3 GB | `gemma3` ❌ | Needs a variant | Sub-Pi-5 tier. Interesting ONLY if a `LoadedModel::Gemma3` arm is added. Great for classify/extract gate-model roles. |
| **Gemma 3 1B** | ~0.7 GB | `gemma3` ❌ | Needs a variant | ~35–45 tok/s on 4 GB phones. Same wiring cost as 270M. |
| **Phi-4 Mini 3.8B** | ~2.7 GB | `phi3` ❌ | Needs a variant + heavy | Smartest-in-class but too big for the phone-node floor; server-tier only. |

## Recommendation (dependency-ordered, cheapest first)

1. **Ship Qwen3-0.6B as the reference/default store model.** It's the single
   most interesting pick that costs *zero code*: dual-mode thinking, tiny, already
   dispatched. Pair 1.7B as the "smart tier" for capable nodes.
2. **Keep SmolLM2-1.7B / Llama-3.2 in the store as llama-arch fallbacks** — they
   load today and give a throughput-optimized alternative for miner nodes.
3. **Add a `LoadedModel::Gemma3` variant only if** a tiny gate/classifier model
   (270M) becomes load-bearing — e.g. the EZKL-viable verifier-model idea from
   scan-07-06 (#5), or a cheap sensor-data classifier. Gemma-270M is the natural
   candidate there. Otherwise defer; every arm is consensus-adjacent surface.
4. **Do NOT chase Phi-4/large models for the phone floor** — they break the
   "phone is a full node" premise. Server-tier `INFERENCE_URL` path only.

## The real blocker isn't the model — it's determinism (unchanged from 07-06)

The **model choice is nearly free** (Qwen3 is already wired). What's *not* free is
making any model's output bit-identical across a heterogeneous phone fleet so the
mining/verification path agrees. That's the batch-invariant-kernels +
OPML-fraud-proof design track (`scan-2026-07-06.md` #5), and it gates
proof-of-inference regardless of which GGUF sits in the store. Picking a fancier
model does not move that needle; picking Qwen3-0.6B keeps the model variable out
of the way while that harder problem is solved.

## Sources
- On-Device LLMs State of the Union 2026 — https://v-chandra.github.io/on-device-llms/
- Best Mobile LLM Models 2026 (Phi-4 Mini / Gemma 3 / SmolLM) — https://www.promptquorum.com/power-local-llm/mobile-llm-models-phi4-gemma-smollm
- Qwen3 Full Lineup Guide 2026 — https://baeseokjae.github.io/posts/qwen-3-full-lineup-guide-2026/
- unsloth/Qwen3-0.6B-GGUF — https://huggingface.co/unsloth/Qwen3-0.6B-GGUF
- Gemma 3 270M GGUF (ggml-org) — https://huggingface.co/ggml-org/gemma-3-270m-GGUF
- Batch-invariant / reproducible inference (nanomaoli/llm_reproducibility) — https://github.com/nanomaoli/llm_reproducibility

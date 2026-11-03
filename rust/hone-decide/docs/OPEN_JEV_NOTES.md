# Open-Jev: how it works (study notes for hone-decide)

Source studied: https://github.com/Zefan-Cai/Open-Jev, commit `bd41188` (MIT, Open-Jev
contributors), cloned on beastly at `X:\ferry\hone-data\hone-models\decide\Open-Jev`.
Files that define the method: `jev/api.py`, `jev/model.py`, `jev/metrics.py`,
`jev/predict.py`, `jev/train.py`, `jev/prefix_cache.py`. Only Open-Jev was studied.
Nothing from TypeSafe's proprietary Jev is used or reproduced.

Released packages (Hugging Face): `ZefanCai/Open-Jev-2B` (sha `0c7aa498`),
`ZefanCai/Open-Jev-9B` (sha `47e96688`), `ZefanCai/Open-Jev-27B-v1.1`. A package holds
only `adapter/` (LoRA), `head.pt`, `temperature.json`, `model.json`; base weights come
from the pinned Qwen revision.

| tag | base (Apache-2.0) | revision | temperature |
|---|---|---|---|
| 2B | `Qwen/Qwen3.5-2B` | `15852e8c16360a2fea060d615a32b45270f8a8fc` | 1.518796342858676 |
| 9B | `Qwen/Qwen3.5-9B` | `c202236235762e1c871ad0ccb60c8ee5ba337b9a` | 1.8969118766347646 |
| 27B-v1.1 | `Qwen/Qwen3.8-27B` | `1d4bf0f2ff6012fd82039f2fa52739d0dd7c60c0` | (not used here) |

## 1. Request -> records (`api.compile_request`)
A request is `{state, questions}`. `state` is text, a JSON object or an array. Each
question has `type` and `instructions` (+ `criteria`):

- **choice**: `criteria` is an ordered map `name -> description|null` (1..255). The
  option shown to the model is `name` or `"name: description"`. Output keys = names.
- **score**: `criteria` is a list of 2..10 level descriptions. Options are the level
  texts; output keys are `"0".."n-1"`; the score answer is `sum(i * p_i)`.
- **noul** (boolean): no candidates. Optional `criteria {true, false}` are appended
  to the question as `"\nYes means: ..."` then `"\nNo means: ..."`. Output order is
  `[false, true]`.

Non-string values are rendered with `json.dumps(v, ensure_ascii=False, sort_keys=True)`
(default separators `", "` and `": "`, Python float repr). hone-decide reproduces this
in `src/pyjson.rs` so the prompt text and the tokens are identical.

## 2. Prompt template (`api.candidate_prompts`)
One independent prompt per candidate; the model never sees other candidates, their
order, the question id or the target:

```
Context:
{state}

Question: {question}
Proposed answer: {option}
Is this proposed answer correct? Answer Yes or No.
```
Noul uses one prompt ending in `Is the answer to this question yes? Answer Yes or No.`
instead of the two `Proposed answer` lines.

The prompt is wrapped by the tokenizer's chat template as a single user turn with
`add_generation_prompt=True, enable_thinking=False`. For Qwen3.5 that is
`<|im_start|>user\n{prompt}<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n`
(verified token-for-token against the Python tokenizer, see PROGRESS.md). No
truncation: inputs longer than `max_length` (4096 in the released packages) are an error.

## 3. Which hidden state the head reads
`AutoModelForImageTextToText` is loaded, then only `full.model.language_model`
(`Qwen3_5TextModel`) is kept; the vision tower, `mtp` and `lm_head` are dropped.
Batches are right-padded. The head reads `last_hidden_state[i, len_i - 1]`: the
**final-normed hidden state of the last real token of each candidate prompt** (the
last token of the generation prompt). Text-only position ids are `0..L-1`, so the
interleaved MRoPE collapses to ordinary RoPE on the first 64 of 256 head dims.

## 4. The head
`nn.Linear(hidden_size, 1, bias=True)` in **float32**, applied to the hidden state
cast to float32. It is initialised from the base model's output embeddings,
`w = lm_head[Yes] - lm_head[No]`, `b = 0` (a Yes-vs-No logit difference), then trained.
2B: hidden 2048; 9B: hidden 4096.

## 5. Scoring and probabilities
- choice/score: logits = `[head(h_c) for each candidate c]`.
- noul: logits = `[0, head(h)]`.
- probabilities = `softmax(logits / T)` with the single saved temperature `T`.
So a candidate's score is independent of the others; normalisation happens only in
the softmax. This is what lets the candidate set be any size and order.

Typed answers (`format_response`): choice -> argmax + `choice_confidence =
(p_max - 1/n) / (1 - 1/n)`; score -> expected level + concentration confidence;
noul -> `p(true)`.

## 6. One pass per candidate vs shared prefix
The reference path (`model.forward`) runs **one full forward per candidate** (batched,
right padded). `--prefix-cache` (`prefix_cache.py`, off by default) computes the exact
token longest-common-prefix once and forks the cache (attention KV **and** the Gated
DeltaNet conv + recurrent states) per candidate; it notes HF rounds the fp32 delta-rule
state to bf16 at a split unless patched. hone-decide always shares the prefix: its
recurrence is the exact per-token rule in fp32, so splitting is exact.

## 7. Training and calibration (`train.py`, `metrics.py`)
- LoRA rank 8, alpha 16 (scale 2), dropout 0, on `q_proj k_proj v_proj o_proj`
  (full-attention layers) and `in_proj_qkv out_proj` (Gated DeltaNet layers). Backbone
  frozen otherwise; head trained with its own LR (lr 5e-5, head_lr 1e-4).
- Loss per record: soft-target cross entropy `-sum q log softmax(z)` plus
  `0.1 * sum (softmax(z) - q)^2` (Brier).
- Temperature: one scalar fitted on a calibration split by cross entropy, log-spaced
  grid over [0.05, 20] then golden-section refinement (`fit_temperature`).
- Metrics: accuracy (hard rows), NLL, Brier, top-label ECE with 15 equal-width bins.

## 8. Qwen3.5 backbone details needed for the Rust port
- RMSNorm is zero-centred: `x * rsqrt(mean x^2 + eps) * (1 + w)`; the Gated DeltaNet
  output norm is plain `* w` followed by `* silu(z)`.
- Full attention (every 4th layer): `q_proj` emits query and an output gate per head;
  per-head q/k RMSNorm; partial RoPE (factor 0.25, theta 1e7); output `* sigmoid(gate)`.
- Gated DeltaNet: `in_proj_qkv` -> causal depthwise conv1d (k=4) + SiLU -> q, k, v;
  q, k L2-normalised, q scaled by `1/sqrt(dk)`; `beta = sigmoid(in_proj_b x)`,
  `g = -exp(A_log) * softplus(in_proj_a x + dt_bias)`; state update per token
  `S = S*exp(g); S += k (beta (v - S^T k))^T; o = S^T q`.
- 2B: 24 layers (18 linear + 6 full), hidden 2048, 8 q heads / 2 kv heads of 256,
  16 k/v linear heads of 128, MLP 6144, vocab 248320.

## 9. What hone-decide keeps and changes
Kept exactly: request types, prompt text, chat wrapping, last-token head, f32 head,
noul `[0, s]`, temperature softmax, LoRA-merged weights (merged offline in fp32,
`py/export_openjev.py`). Changed: small students (SmolLM2) are fully fine-tuned rather
than LoRA-tuned, trained on teacher soft probabilities (KL), and quantised to GGUF.

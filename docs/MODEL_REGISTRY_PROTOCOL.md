# Model Registry Protocol

**Version 0.1 — July 2026**

---

## Overview

HONE is model-agnostic by design. A node does **not** ship with a baked-in model and
must never auto-download one — the operator chooses which model(s) their machine serves,
from the local model store, based on what the hardware can actually run. This is a
sovereignty property: the network picks nothing for you.

But every node facing a model needs the same facts to decide if it can run it — minimum
VRAM, quantization, context length, file size. Having each machine independently analyze
every model is wasteful. So HONE turns model analysis into a **chain primitive**: the
first node to analyze a model publishes its manifest on-chain; every other node reads it
for free; the analyzer earns a small, usage-weighted reward. Useful cataloging is
rewarded; junk nobody runs earns nothing.

The model registry is the discovery input to the capability provisioner
(`hone-provision`) and to Wiiv render workers: "which models fit this machine" is
answered by gating published manifests against detected hardware.

---

## Principles

- **No hardcoded default model.** There is no `const DEFAULT_MODEL` anywhere. A node with
  no model enabled simply reports inference unavailable until the operator enables one.
- **No auto-download of a chosen-for-you model.** Models enter the store by operator
  action (or by an explicitly enabled capability), never silently at startup.
- **Analyze once, share to all.** The first valid manifest for a model is authoritative;
  redundant analysis by later nodes earns nothing and is not required.
- **Subjective quality stays off-chain.** The registry records *objective* facts (size,
  quant, min hardware, context length, a content hash). Whether a model is *good* is the
  operator's / market's call, never consensus.

---

## Entry Types

| Entry type | Who signs | What it does |
|---|---|---|
| `ModelManifestPublish` | analyzer (posting) | Publishes the objective manifest for a model keyed by its content hash: size, quant, min VRAM/RAM, context length, architecture, and the source (CID/URL). First valid publish for a hash wins and becomes authoritative; later duplicates are recorded but earn nothing. |
| `ModelUsageAttest` | job settler (system) | Emitted when a model is used in a settled paid job (inference or render). Increments the model's usage counter, which drives the analyzer's reward. System entry — not user-submitted. |
| `ModelAnalysisReward` | system | Pays the manifest's original analyzer a small, capped share for the epoch's attested usage of that model. Drawn from the existing reward pool (no new issuance). |

Models are keyed by **content hash** (sha256 of the weights), not by name — two nodes
that pulled "qwen2.5:0.5b" from different sources with identical bytes share one manifest;
a renamed or requantized model is a different key.

---

## Model Manifest

The objective facts a node needs to decide if it can run a model, and to gate it against
hardware. Mirrors the `hone-provision` requirements gate.

| Field | Meaning |
|---|---|
| `model_hash` | sha256 of the weights — the registry key. |
| `name` | Human label (e.g. "qwen2.5:0.5b"); advisory, not the key. |
| `size_mb` | On-disk size. |
| `quant` | Quantization (e.g. Q4_K_M, fp16, fp8). |
| `architecture` | Model family (e.g. qwen2, llama, wan2.2-video). |
| `min_vram_mb` / `min_ram_mb` | Hardware gate — what it takes to load/run. |
| `context_length` | Max context window. |
| `modality` | text / image / video / audio (ties into Wiiv render modalities). |
| `source` | CID (preferred) or URL where the weights can be fetched. |
| `analyzer` | Account that first published this manifest. |

A node reads published manifests, gates them against its detected hardware
(`hone-provision::Requirements`), and **suggests** the models that fit. The operator
picks; nothing is auto-enabled.

---

## Analyzer Reward

The incentive to catalog models, kept bounded and sybil-resistant:

- **First valid manifest wins.** Only the first node to publish a correct manifest for a
  `model_hash` is the analyzer of record. Duplicates earn nothing (so there's no race to
  spam-publish).
- **Usage-weighted.** The analyzer earns a share proportional to that model's *attested
  usage in settled paid jobs* during the epoch — analyzing a model that later gets widely
  used earns more; analyzing junk nobody runs earns nothing.
- **Capped per epoch.** Each model's analyzer reward is capped per epoch so a single
  popular model can't dominate the pool.
- **Off the existing pool.** Paid from the standard reward split, **not** new issuance —
  it reallocates a sliver of existing rewards toward useful cataloging, not inflation.

### Anti-gaming

- **Manifest fraud** (publishing wrong requirements to look authoritative) is slashable:
  other nodes that load the model can dispute a manifest whose stated min-hardware is
  contradicted by an actual load, and a disputed-and-confirmed-false manifest forfeits the
  analyzer's stake and reward.
- **Fake usage** is prevented because `ModelUsageAttest` is a **system entry emitted only
  on settled paid jobs** — usage that never happened can't be attested, and a paid job
  leaves a fee trail. You can't inflate a model's usage without actually paying for jobs.
- **Sybil analysis** gains nothing: only the first manifest per hash earns, so registering
  many identities to "analyze" the same model is pointless.
- **Content-hash keying** means you can't earn twice on the same weights under different
  names.

---

## Local Flow (operator side)

```
discover store  →  gate each model vs. hardware  →  suggest fitting models
                →  operator: `hone model enable <name>`  →  node serves it
```

- **Discover**: scan the central model store (`HONE_MODEL_DIR` / the shared store) for
  weight files.
- **Gate**: for each, read its published manifest from chain. If none exists yet, the
  node does **not** analyze it unprompted — analysis + `ModelManifestPublish` happens only
  on an explicit operator action (`hone model analyze <name>` or as part of
  `hone model enable`), consistent with "HONE never acts on models without operator
  intent." Compare `min_vram_mb` etc. to detected hardware.
- **Suggest**: present the models that fit; the operator picks. No default.
- **Enable**: `hone model enable <name>` persists the choice to node config; the embedded
  backend loads that model. `hone model list` shows discovered + fitting models.

The node serves the enabled model until the operator changes it. With nothing enabled,
inference is unavailable — HONE never guesses.

---

## Reserved Accounts

| Account | Purpose |
|---|---|
| `model-registry` | Protocol account for registry bookkeeping; genesis-seeded, no keys. |

---

_Analyze once, share to all. The operator picks the model; the chain remembers what it takes to run._

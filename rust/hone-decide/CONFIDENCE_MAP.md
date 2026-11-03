# hone-decide confidence map (2026-11-03)

The question: what can the deployed model answer confidently enough to act on?

**Confidence** is the top candidate's calibrated probability. **Coverage** is the share of
questions answered at or above a threshold. **t95** is the lowest threshold at which the
answered questions reach at least 95% accuracy (only counted when at least 10 questions are
answered). A group is **high-trust** when its t95 coverage is at least 30%.

- Tool: `py/conf_map.py`. Raw output: grouchly `~/decide/conf_map.json`.
- Test sets: HONE tasks (2000 rows; the 500-row subset for the quantized models) and the
  8 public multiple-choice benchmarks (2400 rows).
- All predictions come from the Rust runtime, except the f32 rows (Python, which matches Rust to about 1e-6).

## Summary table (t95 = threshold / coverage; ★ = high-trust)

| group | 135M-gen Q4 (nebra) | 135M-gen Q8 | 135M-gen f32 | 360M-gen Q8 | 360M-gen f32 |
|---|---|---|---|---|---|
| **HONE, all** | 0.82 / 46% ★ | 0.82 / 49% ★ | 0.84 / 44% ★ | 0.55 / 95% ★ | 0.58 / 91% ★ |
| peer_behavior | 0.57 / 81% ★ | 0.53 / 89% ★ | 0.54 / 89% ★ | 0.30 / 100% ★ | 0.52 / 95% ★ |
| job_routing | 0.75 / 70% ★ | 0.59 / 85% ★ | 0.55 / 91% ★ | 0.76 / 84% ★ | 0.73 / 74% ★ |
| reading_novelty | 0.70 / 39% ★ | 0.71 / 47% ★ | 0.73 / 40% ★ | 0.30 / 100% ★ | 0.30 / 100% ★ |
| sensor_plausibility | 0.96 / 16% | 0.96 / 16% | 0.93 / 30% ★(edge) | 0.71 / 79% ★ | 0.74 / 82% ★ |
| **MCQ, all** | 0.91 / 3% | 0.93 / 3% | 0.94 / 3% | 0.95 / 14% | 0.95 / 14% |
| ARC-Easy | 0.74 / 29% | 0.84 / 28% | 0.91 / 20% | 0.84 / 49% ★ | 0.82 / 50% ★ |
| OpenBookQA | – | – | – | 0.78 / 37% ★ | 0.79 / 36% ★ |
| CommonsenseQA | 0.79 / 4% | 0.75 / 10% | 0.76 / 9% | 0.85 / 22% | 0.85 / 22% |
| ARC-Challenge, PIQA, HellaSwag | – | – | – | 0.94–0.95 / 10–14% | 0.94–0.95 / 11–14% |
| MMLU, BoolQ (yes/no) | – | – | – | – | – |

### By shape (135M-gen Q4, the deployed model)

| shape | t95 / coverage | accuracy on answers at ≥0.9 |
|---|---|---|
| 3 options | 0.83 / 39% ★ | 0.98 (31% covered) |
| 2 options | 0.91 / 9% | 0.94 |
| 4 options | 0.91 / 4% | 0.94 |
| 5 options | 0.79 / 4% | – |
| yes/no (`noul`, BoolQ) | never reaches 95% | 0.67 |
| HONE, prompt < 400 chars | 0.75 / 68% ★ | 0.96 |
| HONE, 400–800 chars | 0.82 / 43% ★ | 1.00 |
| HONE, 800–1200 chars | 0.72 / 30% ★ | 1.00 |
| HONE, ≥ 1200 chars | never | 0.86 |
| MCQ, any length | ≤ 4% | – |

## What it means

1. **The confident zone is short, structured, closed-set decisions over facts in the prompt.**
   - The context is JSON with numbers and ids. There are 2–3 named options with one-line
     definitions. The answer follows from comparing fields (counts, thresholds, presence of a
     flag), not from world knowledge.
   - Peer behaviour, job routing and novelty are already there on the deployed 135M:
     - Peer behaviour answers 81% of cases at 95%+ accuracy (the 0.57 threshold).
     - Routing answers 70% (the 0.75 threshold).
     - Novelty answers 39% (the 0.70 threshold).
2. **Sensor plausibility is the weak HONE task on the small model.** Only 16% clears 95% on the
   135M (at the 0.96 threshold). It needs arithmetic over a series of readings (jumps, physical
   limits). The 360M handles it (79%). Fix on the 135M: put the derived features in the prompt
   (delta vs median, z-score, inside/outside physical range) so the model compares instead of
   computes.
3. **General knowledge is not a 135M job.** On the public benchmarks it rarely clears 95% (≤4%
   coverage). The 360M gets two benchmarks into high-trust (ARC-Easy 49%, OpenBookQA 37%).
   Questions that need facts belong on the 9B teacher (0.874 accuracy) or should abstain.
4. **Yes/no reading comprehension (BoolQ) never gets confident.** The `noul` kind is the worst
   shape at every size. Prefer named 2–3 way choices with definitions over bare yes/no.
5. **Long prompts lose confidence.** HONE prompts over about 1200 chars never reach 95% on the 135M.
   Keep contexts under about 800 chars (about 200 tokens), which is also the fast case on nebra (about 10 s).
6. **Quantization barely matters for the confident zone.** Q4 and Q8 have almost the same
   coverage at 95%. Q4 loses a little on routing (70% vs 85%).

**Caveat:** the HONE tasks are synthetic, labelled by deterministic rules. High trust here partly
means "learned the rule". That is the intended design (cheap labels, and abstentions fall back
to the rule), but real chain data must be checked before trusting the thresholds in production.

## Design rule for HONE features
- Shape the decision as 2–3 named options with one-line definitions.
- Put the relevant fields and pre-computed comparisons in a context under 800 chars.
- Label it from a deterministic rule or the chain.
- Per task, set the abstain threshold at that task's t95 from held-out data.
- Below threshold: answer `abstain` and fall back to the rule, or to the 9B on beastly.

## HONE decision tasks for the confident zone (round 3)

Each task is a 2–3 option choice over structured fields, with a deterministic label source.
- The model answers only when confidence reaches the task threshold.
- Below threshold it abstains, and the caller falls back to the rule or to the 9B.

| task | input (features) | label source | expected volume | cost of a wrong answer | status |
|---|---|---|---|---|---|
| **entry_screen** (accept / duplicate / spam) | entry type, submitter, nonce, data hash, payload bytes, clock skew; submitter's last nonce, recent hashes, entries this epoch, epoch cap; derived flags | nonce, hash-seen, cap, skew and empty checks (chain.rs dedup and nonce guards) | every judgment entry; the node routes all judgment types here in v1 | low in shadow mode. A false "accept" on a replay is still caught by the chain's own nonce guard. | **built**: Q4 100% @ 100% coverage |
| **coverage_usefulness** (new / adds / repeat) | report (reporter, cell, carrier, technology, dBm, dead spot); cell's reporters, distinct count, median dBm, corroborated; derived (already in cell, dB delta, reports this epoch); rules | mirrors chain.rs CoverageReport apply (0.001° grid, 200/epoch cap, 3-reporter corroboration) plus the spec's "pays only for new/changed" | ≤ 200 per reporter per epoch; grows with phones | mis-paid reward (small; first-reporter pays) | **built**: Q4 99% @ 93.9% (test, n=99) / 97.3% (cal) |
| **sensor_plausibility_v2** (yes / no / unsure) | v1 plus derived median, delta vs median, max recent step, in physical range | physical limits plus jump-vs-rate rule from the generator | one per sensor commit (6 channels × 2 phones / 30 s today) | strikes on an honest sensor (false "no"), or junk paid (false "yes") | **built**: Q4 79% @ 98.7% (v1 was 20% @ 100%) |
| sighting_novelty (new / changed / repeat) for birds and BLE | species or rid hash, cell, window, last sightings in cell (count, last window, confidence) | spec §3 novelty: first in (cell, window bucket, metric), or a value outside the noise band | Hone Birds plus BLE sightings, per 15-min window | mis-paid reward | proposed; same shape as coverage_usefulness |
| name_request_wellformed (ok / reserved / malformed) | name length, charset flags, reserved-list hit, popular-list hit | `reserved_names.rs` plus the name grammar (fully deterministic) | low (name NFT mints) | wrong mint (high) | not a model task: the rule is exact and cheap, so the model adds nothing |
| peer_behavior / job_routing / reading_novelty | as round 1 and 2 | generator rules | per peer per epoch / per job / per reading | peer: wrongful strike (high, so it stays shadow until real data) | existing; peer first to activate |

### Round-3 result (135M-r3 Q4, deployed on nebra 17:35)

Thresholds are `thresholds.json` (97% target on cal). Test-set results:

| task | threshold | coverage | accuracy answered |
|---|---|---|---|
| entry_screen | 0.30 | 100% | 100% |
| coverage_usefulness | 0.63 | 99% | 93.9% (n=99; cal 97.3%) |
| sensor_plausibility_v2 | 0.87 | 79% | 98.7% |
| peer_behavior | 0.62 | 73% | 96.9% |
| reading_novelty | 0.71 | 38% | 98.0% |
| job_routing | 0.96 | 30% | 100% |
| sensor_plausibility (v1) | 0.94 | 20% | 100% |

- **Regression:** HONE overall .824 → .820 and MCQ .510 → .509, both within 1 point. Peer by itself fell
  .910 → .851, but at its threshold it still covers 73% at 96.9%.
- **nebra over HTTP (live service, 21 jury items):** 0 errors, 21/21 reveals verified, 29.7 s p50 /
  35.4 s p90 per vote, RSS 140 MB, cgroup peak 169 MB (cap 250 MiB), min free memory 506 MB,
  hone-node untouched.

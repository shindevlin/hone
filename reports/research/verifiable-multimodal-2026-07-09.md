# HONE Verifiable Multi-Modal Inference — Design Scan 2026-07-09

**Verification-first synthesis.** Four design angles were proposed for how HONE judges *"reasonable, not right"* on non-identical, multi-modal model output, then each was adversarially checked against the live `rust/hone-node/src/inference.rs` state machine and against 2026 sources (LLM-judge attacks, RLVR verifiers, ASR/VLM metrics, candle model support). Every mechanism was scored on whether it **HOLDS**, its strongest objection, and its fix. A parallel model catalogue was verified for candle-loadability. Verdicts below: **SHIP-NOW** = build in v1, sound as-stated · **REFRAME** = holds only after a stated re-architecture · **GATE** = blocked on a prerequisite that must land first · **DOC-ONLY** = a hardline/prohibition to write down, not code.

The single organizing truth, confirmed against live sources and the founder constraint: **re-execution equality is dead** (2509.11068 replicability and 2509.24257 VeriLLM both *require* identical hardware/software — outputs diverge RTX 4000 Ada vs A40 on floating-point alone). So HONE's answer is a **Gate → Judgment** pipeline whose deterministic tier carries the real anti-fraud weight and whose subjective tier must be honestly de-scoped from "discovers truth" to "bounded-loss market."

---

## Executive summary — the highest-leverage findings

Ranked by objective impact (correctness of the verdict pipeline first, then sovereignty/anti-cartel, then cost-to-ship):

| # | Finding | Layer | Verdict | Why it's #-ranked |
|---|---------|-------|---------|-------------------|
| 1 | **The collapse map is the whole strategy: route jobs so an OBJECTIVE cheap checker replaces the board wherever a referent exists** | Verification design | SHIP-NOW | Code-with-tests, math-with-checkable-proof, structured/tool-call, and ASR-with-reference collapse to Tier-1 deterministic checks. For these the board is optional spot-check. This is the cheapest path to *verifiable* multi-modal jobs and it holds under adversarial review. |
| 2 | **Live bond bug: verifiers vote with ZERO stake at risk — dissenter-slashing is a runtime no-op** | Economics | SHIP-NOW | `apply_bid` gates verifiers on `rep.score` only (never locks balance); the dissenter slash does `slash.min(get_stake(dissenter))` = `min(x,0)=0` for a fresh sybil. Sybil boards are literally free today. A locked `VERIFIER_BOND` at commit time is a standalone bug fix, independent of everything else. |
| 3 | **Two consensus defenses are BUILT-BUT-DARK: `VERIFIER_ASSIGNMENT_ENABLED=false`, `INFERENCE_COMMIT_REVEAL_ENABLED=false`** | Economics | GATE | Random assignment + commit-reveal code paths exist and are gated behind `const false`. Flipping them is a prerequisite for optimistic/challenge-only convening (a small, adversarially-selected board is trivially self-assignable while assignment is off). |
| 4 | **Dissenter-slashing means TRUTH on collapsible classes but CONFORMITY on subjective ones — do not conflate them** | Verification design | REFRAME | Slashing a verifier who voted against a green unit-test suite is provably correct. Slashing a lone dissenter on freeform text/art/TTS-naturalness punishes the honest minority against a possibly-fooled majority. Slash magnitude must scale with checker confidence; on zero-referent classes the floor is low. |
| 5 | **Judge-model MONOCULTURE inverts the incentive on subjective jobs** | Verification design | REFRAME | Economic pressure drives every verifier to the same cheapest-adequate small judge → correlated blind spots + shared master-key susceptibility. Commit-reveal is blind to correlated *independent* error. Require ≥K distinct judge-model content-hashes before a subjective quorum is slashing-eligible — turning the model marketplace into a security asset. |
| 6 | **Optimistic attestation is an INCREMENT, not an inversion — the code is already optimistic-by-accident** | Economics | REFRAME | `apply_pay` already pays from `Completed` (comment: "pays jobs that expired without verifiers"); verification is a permissionless pull-loop (`main.rs:1231`), not a mandatory per-job board. Add a worker bond + a real challenge entry + a genuine `jobs_challenge_expired()` sweeper. Drop the false "board is mandatory on 100%" premise; two of its extension anchors (`auto_settle_unverified`, `inference.rs:285`) are fictional. |
| 7 | **Ground-truth "traps" only bite the measure-zero checkable subset — do not market them as closing plausible-but-wrong on open-ended jobs** | Verification design | DOC-ONLY | A poster-known trap answer *is* ground truth, which the iron constraint says doesn't exist for real generation jobs. Traps discipline classification/extraction/factual work; they give ZERO coverage on open-ended generation. Honest scoping, not a mechanism. |
| 8 | **Big-model tier gating claims a VRAM-capacity primitive that does not exist in code** | Scaling | GATE | `inference.rs` has zero refs to hardware/min_vram/capability; `hardware.rs` has identity (gpu_serial/machine_id) but NO VRAM/RAM detection. "Registry hardware facts decide who verifies" is unbuildable until a signed capability report + random capability re-audit ship. |

**Cross-cutting decision that gates the subjective tier:** you cannot simultaneously **hide** a requester's reference (anti-overfit) and **expose** it to the board (poison-detection) on a chain with no trusted party. Resolve by moving the **fee to escrow**, not the verdict — poisoning becomes +EV-negative without any pre-inspection. Decide this before shipping any requester-supplied-reference collapse.

---

## 1. Verification design — judging "reasonable, not right"

The frame all four angles converge on: a verifier runs a **two-tier pipeline** *before* it emits its existing `InferenceJobVerify{verdict, value_score, reason}`. **Tier 1** = cheap deterministic GATES that can only *fail* a job (run identically on every board member, so honest nodes agree despite divergent model output). **Tier 2** = subjective reasonableness judgment, only on outputs that clear every gate, feeding the EXISTING commit-reveal + weighted quorum. The three-valued verdict `{approved, rejected, review_required}` is unchanged; the rubric only decides *which* verdict a verifier reveals.

### 1a. Reasonableness rubric + collapse map

| Design angle | Mechanism | Extends existing (inference.rs) | New surface | Adversarial verdict |
|---|---|---|---|---|
| **Tier-1 deterministic gates** | Per-`verify_class` pure checks: UTF-8/schema/max-tokens/n-gram-repetition (text); parse+compile+hidden-test-suite (code); CAS/Lean/SMT (math); schema+enum (structured); decode+resolution+perceptual-hash+NSFW (image); WER-vs-reference (ASR) | Nothing replaced — gates run *off-chain*, only the verdict lands via `InferenceJobVerify`. `reason` field (entry.rs:557, currently unused) carries the structured rubric receipt | ONE consensus field: `verify_class: String` (defaults `text_freeform`). Optional requester-posted artifact hashes: `test_suite_hash`, `canonical_answer_spec`, `reference_transcript_hash`, `output_schema` | **HOLDS.** This is the design's real contribution. Determinism claims survive *if* the failure-mode-6 caveat is a hard invariant: any non-bit-reproducible gate (GPU float drift, sandbox flakiness) is DOWNGRADED to a Tier-2 signal, never a hard gate. |
| **Tier-2 subjective judgment** | text: coherence/on-task/format · image-video: VQAScore-style VLM-as-judge (CLIPScore fails compositional prompts) + artifact detection · TTS: ASR round-trip intelligibility (semi-objective bridge) + prosody vote · math-no-checker: unit/dimension/magnitude sanity | Feeds the EXISTING `value_score: u64` and the weighted quorum `resolve_quorum_verdict`; dispute→human `MIN_REVIEW_VOTES=3` untouched | WER bands (`WER_AUTO_APPROVE_BPS≈600`, `WER_AUTO_REJECT_BPS≈1500`), `VQA_ADHERENCE_MIN`, review_required buffer zones around every band edge | **REFRAME.** Holds only as "kill garbage." The paper over-claims a lone taste-vote "loses cleanly" — it loses to a *coordinated or monoculture* majority just as surely as to an honest one, and the chain cannot tell them apart. |

**The collapse map (the key insight — see §3):**

| Class | Referent? | Tier that decides | Board role |
|---|---|---|---|
| code + tests, math + machine-checkable proof, structured/tool-call, ASR + reference transcript | **Objective** | Tier 1 checker authoritative | Optional anti-gaming spot-check |
| freeform text, image-gen, TTS | **Partial** | Tier 1 gates format/validity/policy; Tier 2 judges on-task/adherence/naturalness | Required for the subjective slice |
| open-ended creative text, artistic image/video aesthetics, TTS naturalness, unreferenced translation/summary | **None** | Tier 1 strips garbage only | Full weighted quorum, irreducibly subjective |

### 1b. Adversarial verdicts on the two subjective attacks

**Judge monoculture = honest-but-wrong consensus (strongest objection to the rubric).** Verified: *One Token to Fool LLM-as-a-Judge* (2507.08794) — a bare `":"` gives up to 35% FPR on GPT-4o, up to 80% token-augmented, effect holds on Claude-4 and Qwen2.5-72B. If 4/5 verifiers share a judge family they share the master-key susceptibility; a worker's control token flips the *correlated majority* to `approved` and the one honest dissenter is slashed. **Commit-reveal does not save this** — it stops *copying*, not *correlated independent error*. The design leans on commit-reveal here and thereby misdiagnoses the threat.

**Fix (REFRAME):** (1) carry the judge model's registry content-hash in the verify receipt; require ≥K distinct judge-model hashes before a subjective quorum is slashing-eligible (correlated single-model boards may vote but cannot slash). (2) Demote subjective-class dissenter-slashing to a bounded, bilateral, `VERIFIER_SUBJECTIVE_SLASH_BPS ≪ collapsible-slash` prediction market keyed to the *settled* outcome, not the same-epoch majority. (3) Master-RM-harden judges as a registry-attested, manifest-fraud-slashable property. (4) Resolve poison/hide by **escrowing the fee** on requester-reference auto-reject.

**verify_class as unguarded privilege escalation.** Making the class requester-declared and guarding it with "a verifier may vote review_required if inapplicable" protects the one new consensus surface with the very subjective tier it was trying to avoid. **Fix (SHIP-NOW):** make the SAFE default the expensive one (`text_freeform`), and make *downgrading* to a cheaper collapsible class require posting the corresponding checker-artifact hash at post time. No `test_suite_hash` ⇒ cannot claim code auto-approve. This turns a soft guard into a hard precondition and closes the only direction with an attack payoff.

**Separation — already built in `inference.rs` vs new work:**

- **Already built:** three-valued verdict + `resolve_quorum_verdict` weighted quorum; commit-reveal (`apply_commit`/`apply_verify`); random `VERIFIER_ASSIGNMENT` (dark); rubber-stamp leaky-bucket → weight-halve/suspend; dissenter slashing (`compute_verifier_split`); `value_score`; `reason` field; auto-scaled 1/3/5 board; optimistic dispute → `MIN_REVIEW_VOTES` human review; MODEL_REGISTRY `modality` field → `verify_class` mapping.
- **New work:** `verify_class` field + artifact-hash preconditions; WER/VQA bands + buffer zones; judge-model-diversity quorum eligibility; confidence-scaled + bilateral subjective slashing; fee-escrow on reference auto-reject; downgraded-gate-is-Tier-2 hard invariant.

---

## 2. Staked-attestation + optimistic-dispute economics

| Knob | Proposal | Extends existing | Adversarial verdict |
|---|---|---|---|
| **Worker attestation bond** | Lock `max(job_fee, MIN_ATTEST_BOND)`, requester multiplier `k∈{1,2,4}`; released on Verified | Same `get_stake`/`set_stake` escrow used for dissenter slash; fold `attest_bond` into `InferenceJobComplete` | **HOLDS**, but MUST be escrowed *atomically at Complete* (apply_pay reads live `get_stake`; a worker could unstake during the 10-min window). |
| **Challenge window** | Reuse `CLAIM_WINDOW_EPOCHS=20` (10 min); auto-promote unchallenged → Verified | **Re-anchor:** attach to the per-node review loop (`main.rs:1231-1299`) + `build_pay_entry_happy` (`inference.rs:958`) + the Completed-allowed branch of `apply_pay` (`:786`). Add a REAL `jobs_challenge_expired()` modeled on `jobs_claim_expired` (`:285`) | **REFRAME.** The named sweeper `auto_settle_unverified` **does not exist** and `:285` is `jobs_claim_expired`, not an auto-verify. The "board is mandatory on 100%" premise is false. |
| **Challenger bond + bounty** | Symmetric `challenge_bond = worker_bond`; loser pays winner; `CHALLENGE_BOUNTY_BPS≈5000`, remainder → `RECYCLE_FUND_ACCOUNT` | Routes losing worker into existing `build_pay_entry_disputed` / Rejected path; convenes existing board | **HOLDS.** Bond geometry is correct optimistic-rollup fraud-proof shape; genuinely -EV for cheating workers and frivolous challengers. |
| **Compose w/ dissenter-slash + leaky-bucket** | Verifier pool on dispute funded from LOSER's bond, not requester fee; leaky-bucket keys per-verdict (already does) | `split_board_by_consensus`, `update_verifier_approval_rate` untouched | **HOLDS** with caveat: challenge-only convening = sparse, adversarially-selected samples → lower/replace the epoch-count window semantics before flipping the flag, else stale rubber-stamp scores false-suspend honest verifiers. |

**New surface:** `InferenceJobChallenge{job_id, challenger, bond, reason_hash, epoch}` (genuinely new — enters Disputed from *outside* the board); `#[serde(default)]` JobState fields (`attest_bond`, `challenger`, `challenge_bond`, `challenge_window_end`); constants `MIN_ATTEST_BOND`, `CHALLENGE_BOUNTY_BPS`, `OPTIMISTIC_MODE_ENABLED: bool` (ships dark, mirrors the two existing `const false` flags). Block-0 hash unaffected (feature-gated); testnet re-smoke required before flip.

**Hard launch gate (GATE):** `VERIFIER_ASSIGNMENT_ENABLED=true` is a *prerequisite*, not a nicety — challenge-only convening shrinks and adversarially selects the board, so unassigned/self-selected verifiers become the dominant attack. Address k-multiplier underpricing with a default bond schedule tied to `max_fee` in `apply_post` (`:323`), not naive-requester trust.

**Honest reframe:** this converts *free volunteer* review into *incentivized* review (today a garbage worker is only caught if some idle node happens to look). That is the real, defensible value — an increment on an already-optimistic layer, not a core inversion.

---

## Sybil / collusion resistance (three-pillar hardening)

| Pillar | Diagnosis (verified against live code) | Verdict |
|---|---|---|
| **(a) Entry bond** | `apply_bid` (`:360-400`) gates verifiers on `rep.score` only, never locks stake; slash = `slash.min(get_stake(dissenter))` = no-op at stake 0. **Sybil boards are free TODAY.** | **SHIP-NOW.** Standalone bug fix: lock `VERIFIER_BOND` at commit, debit the *locked escrow key*, not free-floating `get_stake`. Also flip the two dark flags. |
| **(b) Reasoned-dissent escalation** | Activate the unused `reason` field; a slashed dissenter posts bond → forces existing Disputed→Claimed→Reviewed; if upheld, invert `dissenter_slashes` onto the majority (UMA-style escalation game) | **REFRAME.** Reason-quality is itself subjective and the human layer is sybil-able — relocates trust, does not close it ("turtles"). |
| **(c) Ground-truth traps** | Poster-committed `sha256(answer\|salt)`, indistinguishable via same epoch-entropy assignment; approving a trap-failing output is provably wrong → hard slash | **DOC-ONLY.** A trap *is* ground truth, which the iron constraint denies for real generation. Coverage → 0 on open-ended jobs. Discipline classification/extraction/factual work only; do not claim it closes plausible-but-wrong in general. |

**Residual (unfixable at protocol level):** a deep-pocketed cartel controlling many *distinct machines* out-bonds honest challengers (the real March-2025 UMA 25%-voting-power Polymarket attack). Bound exposure instead: cap per-job value that pure reasonableness-review can settle; route high-value/non-checkable jobs to a slower path (larger boards, mandatory reasoned dissent, human circuit-breaker) — explicitly trading latency for safety. The honest deliverable is **bond + activated defenses + bounded exposure**, NOT a claim that the majority-cartel-on-non-checkable-output attack is defeated.

---

## Big-model & heavy-modality scaling (verification-cost ladder)

A **tiered `verify_tier: u8` (0-3)** on JobState — a tier only changes what a verifier *computes* before pushing its verdict; the board/quorum/dispute FSM is untouched, Tier 3 IS the existing `review_required→Disputed→MIN_REVIEW_VOTES` path relabeled.

| Tier | Check | Cost | Verdict |
|---|---|---|---|
| **0 — format/artifact/manifest/model-hash** | Valid container, decodable frames, token count, model content-hash match; video temporal-consistency scorers; pLDDT recompute (ESMFold 10-30x cheaper than AlphaFold-class) | O(1), phone-runnable | **HOLDS — the one genuinely sound rung.** Catches lazy/wrong-model/cost-evasion without re-running the big model. `TIER0_ARTIFACT_REJECT_WEIGHT` (objective fails weigh more) is correct. |
| **1 — small-judge reasonableness** | 7B-class judge reviews a 405B output; board members are the ensemble | one small forward pass ≪ 405B | **REFRAME.** Quorum as coded is *unweighted* majority — the exact baseline Weaver (2506.18203) beats by 13.5%; citing Weaver as the hardening misappropriates it. |
| **2 — checkable-subproblem / proof-carrying** | Run tests / check Lean proof (VERINA, proof-carrying output); sample k-of-n sub-claims | verify a certificate | **HOLDS** where a certificate exists; must NOT be "optimized" into token-level re-execution. |
| **3 — capacity-matched full re-review** | Verifier meeting `min_vram` re-reviews | full cost, rare-by-design | **GATE.** At the true frontier there is no capacity-matched second party → Tier 3 is empty and escalation terminates in the same fooled small-judge pool. |

**Strongest objection (soundness inversion):** for the expensive jobs where verification matters most, the check is weakest (small judge) *and* the incentive layer (unweighted quorum + dissenter-slash + no diversity enforcement) actively drives honest verifiers toward correlated failure and slashes the one who runs a better judge. It bounds COST but cannot manufacture GROUND TRUTH.

**Fixes:** (1) BUILD the missing VRAM/RAM capacity primitive in `hardware.rs` + a mandatory random Tier-3 capability re-audit that slashes on mismatch — until then, document min_vram gating as self-attested-plus-audited, *not* on-chain-enforced. (2) ENFORCE judge-model diversity in `VERIFIER_ASSIGNMENT`. (3) Replace unweighted quorum with reputation/accuracy-weighted quorum on Tier-1/2 and STOP slashing minority dissent on subjective tiers. (4) DOC-ONLY hardline (parallel to no-offline-mode): forbid smuggling in token-level re-execution — TAO (2510.16028) does tolerance-region re-exec across heterogeneous HW but still needs the same weights loaded, so it does NOT rescue phone-verifies-405B.

---

## 2. Model catalogue — candle-loadability + extension roadmap

`candle_loadable` legend: **today** (loads in HONE's existing candle dispatch) · **needs-variant** (candle ships a loader; HONE must wire a dispatch arm) · **needs-new-modality** (new arch module and/or quant kernel) · **external-only** (route via `INFERENCE_URL` worker near-term).

### Audio

| Model | Tier | candle | License | Verdict | Note |
|---|---|---|---|---|---|
| Whisper large-v3-turbo | med-low-vram | **today** | MIT | **SHIP-NOW STT default** | First-class `whisper` module; cleanest license in the set; 99 langs. |
| Whisper tiny/base/small | phone | **today** | MIT | **SHIP-NOW phone STT** | CPU-only, sub-second on mobile — fits "phones are full nodes." |
| CSM / MetaVoice / Parler-TTS | med-low-vram | **today** | Apache-2.0 | **SHIP-NOW TTS proof** | Existing `csm`/`metavoice`/`parler_tts` modules — HONE can do TTS today. |
| Kyutai TTS 1.6B | med-low-vram | **today** | CC-BY-4.0 / Apache | **DEP** | Official Rust/candle server; Mimi codec is a candle module; most candle-native TTS. |
| Codecs EnCodec/Mimi/SNAC/DAC | phone | **today** | permissive | **KEY LEVERAGE** | All four are candle modules — the reusable "back half" of any audio generator. |
| Distil-Whisper v3.5 | med-low-vram | **needs-variant** | MIT | **LOW-effort** | Whisper module + decoder-layer config. |
| **Kokoro-82M** | phone | **needs-variant** | Apache-2.0 | **HIGHEST-ROI extension** | Tiny StyleTTS2-class port; best phone TTS; proven Rust demand (Kokoros). |
| Dia-1.6B | med-low-vram | **needs-new-modality** | Apache-2.0 | **MED** | Backbone port → wire to existing candle `dac` (codec reuse halves work). |
| Parakeet-TDT-0.6B | med-low-vram | **needs-new-modality** | CC-BY-4.0 | **HIGH** | Tops Open ASR leaderboard, but FastConformer + TDT transducer = biggest STT build. |
| MusicGen small/medium/large | server | **needs-new-modality** | **CC-BY-NC-4.0** | **GATE (non-commercial)** | Reuses candle `encodec`; weights NON-COMMERCIAL — gate off paid jobs. |
| Stable Audio Open 1.0 | gpu-only | **needs-new-modality** | Stability Community (<$1M) | **external-only** | Latent-diffusion DiT audio sampler; license gate must be a registry fact. |
| ACE-Step 1.5 | gpu-only | **external-only** | Apache-2.0 | **external adapter** | Best permissive music option; candle port is a large research effort. |

**Roadmap:** ship STT now (turbo + tiny) → port Kokoro (phone TTS win) → Kyutai via existing infra → decide Parakeet (best STT, hard) vs Dia (best permissive TTS, medium) → keep diffusion music external. Add audio manifest fields: `sample_rate`, `codec`, `max_audio_seconds`, `streaming`, `voices/langs`, and enforce license as a first-class registry fact.

### Video + Image

| Model | Tier | candle | License | Verdict | Note |
|---|---|---|---|---|---|
| SDXL 1.0 / SD-turbo | med-low-vram | **today** | Stability Community | **SHIP-NOW image** | UNet SD path in candle core; ecosystem king (LoRAs/ControlNet). |
| BLIP captioning | phone | **today** | BSD-3 | **DEP (weak)** | Zero-work caption baseline for image-understanding verification signals. |
| **Qwen3-VL 2B/4B/8B** | phone | **needs-variant** | Apache-2.0 | **HIGHEST-leverage VLM** | Reuses HONE's existing qwen2/qwen3 text dispatch; gap = vision encoder + mmproj. Directly powers "REVIEW the output" verification of image/video jobs. |
| **Z-Image Turbo 6B** | med-low-vram | **needs-variant** | Apache-2.0 | **DiT anchor** | Strongest low-VRAM Apache image model; build the DiT+VAE+scheduler pipeline ONCE here. |
| FLUX.2 [klein] 4B | med-low-vram | **needs-variant** | Apache-2.0 | **DEP** | Best commercial-safe consumer image model; reuses the Z-Image DiT scaffold (~70% shared). |
| Qwen-Image / Edit 20B | server | **needs-variant** | Apache-2.0 | **DEP** | Best text-rendering + editing; directly relevant to Wiiv manipulation jobs. |
| FLUX.2 [dev] 32B | server | **needs-variant** | **NON-commercial** | **STUDY** | Quality leader but license-restricted — reference only. |
| **HunyuanVideo 1.5** | med-low-vram | **needs-new-modality** | Apache-2.0 | **VIDEO anchor** | Strongest fully-open video fitting consumer GPUs (6-14GB); build video primitives here first. |
| Wan 2.2 TI2V-5B / A14B | med-low-vram | **needs-new-modality** | Apache-2.0 | **DEP** | First MoE video diffusion; reuses the Hunyuan 3D-VAE scaffold + MoE routing. |
| LTX-2 19B | server | **external-only** | LTX Community (<$10M) | **external adapter** | Only open native-4K + SYNCED-AUDIO — feeds Wiiv audio+video; existing external `candle-video` crate is the on-ramp. |

**Candle roadmap:** (a) ship SDXL image today; (b) Qwen3-VL understanding via existing qwen path; (c) build the **DiT image pipeline once** anchored on Z-Image, then FLUX.2-klein + Qwen-Image reuse it; (d) build the **new video modality** (3D causal VAE + temporal DiT + scheduler) anchored on HunyuanVideo 1.5, borrowing from the external `candle-video` crate. Video is candle's true gap — a multi-week first lift, incremental thereafter.

### Math + Science (the objective-collapse goldmine)

| Model | Tier | candle | License | Verdict | Note |
|---|---|---|---|---|---|
| Qwen2.5-Math 1.5B/7B/72B | med-low-vram | **today** | Apache (72B: Qwen lic) | **GOLD — TIR** | Emits Python a sandbox re-runs → "reasonable" collapses to "checkable." qwen2 arch. |
| Qwen2.5-Coder 0.5B→32B | phone→server | **today** | Apache (3B: Qwen lic) | **GOLD — compile/test** | Code that compiles/passes tests is objectively checkable. Full phone→server ladder. |
| Kimina-Prover-Distill 1.7B/8B | phone | **today** | Apache-2.0 | **GOLD — Lean, phone-tier** | Lean-4 proofs, compiler-checked; 1.7B GGUF runs on a phone node. |
| Goedel-Prover-V2 8B/32B | med-low-vram | **today** | Apache-family | **GOLD — Lean** | 32B ≈ 88-90% MiniF2F at 80x smaller than 671B rivals; self-correction = built-in dispute loop. |
| Qwen3 dense 0.6B→32B | phone→server | **today** | Apache-2.0 | **DEP — reasoner** | General reasoner emitting TIR/Lean for the checker to verify. |
| AceMath-Instruct + RM | med-low-vram | **today** | **CC-BY-NC** | **verifier-reference only** | Reward models useful as an automated reasonableness scorer — but non-commercial. |
| DeepSeek-Prover-V2 7B/671B | med-low→server | **needs-variant** | MIT | **GOLD (671B = MoE variant)** | Lean 4, machine-checked; 671B needs candle MoE dispatch. |
| Qwen3-Coder-Next 80B-A3B | server | **needs-variant** | permissive | **DEP** | 3B active = cheap agentic coder; qwen3-moe dispatch work. |
| ChemLLM-7B / nach0 | med-low-vram | **needs-variant** | Apache-ish / open | **STUDY** | RDKit-checkable SMILES, but NOT robust to SMILES variation → dispute risk. |
| ESM3-open / ESMFold | gpu-only | **needs-new-modality** | **Cambrian NON-commercial** | **external / research** | Structure checkable (TM-score) but protein-modality path + license both block commercial nodes. |
| Boltz-2 | gpu-only | **external-only** | **MIT** | **external adapter** | AF3-class, structure+affinity, ~1000x faster than FEP; MIT is the bio standout — dream verification target. |

**Key math-science leverage:** the "gold" (Lean provers + code + TIR math) needs **ZERO candle work** — it's all Qwen-family GGUF that loads today. The real build is the **off-model CHECKER harness**: bundle a Lean 4 compiler + sandboxed code runner as HONE verifier plugins so a verdict = "checker returned pass," not a judged "reasonable." That is exactly the objective collapse the founder wants.

### Big models on low VRAM

| Model | Tier | candle | License | Verdict | Note |
|---|---|---|---|---|---|
| Qwen3-32B dense | med-low-vram | **today** | Apache-2.0 | **SHIP-NOW default** | Rivals 70B, fits single 24GB; loads via existing qwen3 arm, zero code change. |
| Llama-3.3-70B | server | **today** | Llama Community | **DEP (license flag)** | Loads via existing llama arm; single-24GB path is IQ2/IQ3 with quality loss. |
| **Qwen3-30B-A3B** (+3.6-35B-A3B) | med-low-vram | **needs-variant** | Apache-2.0 | **HIGHEST-value MoE** | candle ships `quantized_qwen3_moe.rs`; HONE dispatch has NO `qwen3moe` arm → hits `bail!()`. ONE ~30-line variant unlocks the entire small-active-param MoE tier. |
| Gemma 3 27B | med-low-vram | **needs-variant** | Gemma (restricted) | **DEP (license flag)** | candle ships `quantized_gemma3.rs`; HONE has no gemma3 arm (candle issue #3215). |
| Mistral-Small-3.2-24B | med-low-vram | **needs-variant** | Apache-2.0 | **DEP** | Best Apache dense fitting 16GB; `quantized_mistral.rs` ships, verify metadata routing. |
| Qwen3-235B-A22B | gpu-only | **needs-variant** | Apache-2.0 | **STUDY (VRAM myth)** | 22B active but ALL 235B resident (~120GB) — registry `min_vram` MUST reflect total weights, not active params. |
| gpt-oss-20b / 120b | med-low→gpu | **needs-new-modality / external** | Apache-2.0 | **HARD** | No gpt-oss loader AND no MXFP4 dequant kernel — new arch + new quant format. |
| BitNet b1.58-2B-4T | phone | **needs-new-modality** | MIT | **phone dark-horse (hard)** | Native 1.58-bit ternary; needs packed-ternary kernel candle lacks. Runs on ARM SBC ~1.2GB, no GPU. |
| DeepSeek-V3.2 / V4-Flash | gpu-only | **external-only** | MIT | **bound the space** | MLA + fine-grained MoE, no candle loader; only V4-Flash approaches workstation territory. |

**Candle gap, two buckets:** **CHEAP** — Gemma-3-27B, Mistral-Small-24B, and **qwen3-moe** are each ~20-40 lines of dispatch (the documented `(a) variant, (b) load arm, (c) forward arm` pattern already proven for qwen2/qwen3); candle already ships the loaders. **qwen3_moe is the single highest-value arm** — one variant unlocks 30B-A3B + 35B-A3B, the whole "big capability, low VRAM" thesis. **EXPENSIVE** — gpt-oss (MXFP4 kernel), BitNet (ternary kernel), DeepSeek/GLM (MLA) are new-backend ports, external-only today. Note the task premise error: candle ships `quantized_qwen3_moe.rs` but there is **no `quantized_qwen2_moe.rs`** in candle main.

---

## 3. The key insight — objective-collapse ordering is the cheapest path to shipping

The single call-out that orders everything: **which modalities collapse "reasonable" into an objective cheap check vs which stay genuinely subjective board votes.** This is not a nuance — it is the *sequencing strategy*, because for collapsible classes dissenter-slashing has an objective referent (it *works*), and for non-collapsible classes it silently re-creates the very majority-imposes-a-state-the-minority-cannot-reconcile failure the chain was built to avoid.

**Fully collapsible → ship first, board optional (a deterministic checker is authoritative):**
- **code + requester tests** → sandbox parse/compile/test (SandboxFusion / RLVR practice)
- **math + canonical answer or machine-checkable proof** → CAS / Lean / SMT (DeepSeek-Prover, Kimina, Goedel — all candle-loadable today)
- **structured / tool-call** → schema + enum + referential validation
- **ASR + reference transcript** → WER (exact edit-distance metric; Whisper is candle-loadable today)

These four are where the math-science catalogue and the verification design *fuse*: the models exist in candle now, and the checker harness (Lean compiler + sandbox) is integration, not modeling. **This is the cheapest verifiable-multimodal slice on the board — ship it first.**

**Partially collapsible → hard slice objective, rest subjective-but-assisted:** freeform text (format objective, on-task subjective), image-gen (validity/policy objective, VQAScore-assisted adherence), TTS (round-trip intelligibility semi-objective, naturalness subjective).

**Genuinely subjective → irreducible board votes, must be REFRAMED as bounded-loss market:** open-ended creative text, artistic aesthetics, TTS naturalness, unreferenced translation/summary. Tier-1 removes garbage; beyond that the honest deliverable is judge-diverse, escrow-backed, confidence-scaled bilateral loss — **not** cryptographically-certain punishment of the honest minority.

**The ordering IS the strategy:** build left-to-right on this axis. Every job you can push toward the objective end is a job whose slashing is legitimate, whose disputes are cheap, and whose determinism claims survive a fork-averse chain. The subjective end is where you spend the hard security budget — so ship it last and ship it de-scoped.

---

## 4. Dependency-ordered adoption sequence

Objective, dependency-ordered — correctness/live-bugs first, then the collapsible slice, then subjective hardening:

1. **Fix the live bond bug** — lock `VERIFIER_BOND` at commit; debit the locked escrow key, not `get_stake`. *(standalone; makes dissenter-slashing non-theatre)*
2. **Flip the two dark flags** — `VERIFIER_ASSIGNMENT_ENABLED` + `INFERENCE_COMMIT_REVEAL_ENABLED` → true; re-smoke testnet. *(prerequisite for everything adversarial; block-0 unaffected)*
3. **Wire `verify_class` + artifact-hash preconditions** — the ONE new consensus surface; safe default `text_freeform`; downgrading requires posting the checker-artifact hash. *(gates the collapse)*
4. **Ship Tier-1 for the fully-collapsible classes** — bundle the Lean 4 compiler + sandboxed code runner as verifier plugins; wire ASR WER + structured schema checks. Models load today. *(the cheapest verifiable-multimodal slice; §3)*
5. **Add the candle qwen3-moe dispatch arm** — one ~30-line variant unlocks Qwen3-30B/35B-A3B (big capability, low VRAM). *(highest-value model wiring)*
6. **Qwen3-VL via existing qwen path** — vision encoder + mmproj → powers "REVIEW the output" verification of image/video jobs. *(unblocks visual verification)*
7. **Optimistic attestation increment** — worker bond + `InferenceJobChallenge` + real `jobs_challenge_expired()` sweeper anchored on the actual pull-loop; ships dark behind `OPTIMISTIC_MODE_ENABLED`, gated on step 2. *(incentivized review)*
8. **REFRAME the subjective tier** — judge-model-diversity quorum eligibility (≥K distinct registry hashes), confidence-scaled bilateral `VERIFIER_SUBJECTIVE_SLASH_BPS`, fee-escrow on reference auto-reject, Master-RM-hardened registry-attested judges. *(closes monoculture inversion; do deliberately)*
9. **Build the missing VRAM capacity primitive** + random Tier-3 capability re-audit; enforce judge diversity in assignment; replace unweighted Tier-1/2 quorum with weighted quorum. *(makes big-model tier gating real instead of self-attested)*
10. **Candle modality builds** — DiT image pipeline once (anchor Z-Image → reuse for FLUX.2-klein/Qwen-Image); then the new video modality (anchor HunyuanVideo 1.5). Kokoro TTS port slots in parallel as the phone-audio win. *(the multi-week research track)*

**DOC-ONLY hardlines to write down (parallel to no-offline-mode):** (a) any non-bit-reproducible gate is DOWNGRADED to a Tier-2 signal, never a hard gate; (b) forbid "optimizing" Tier-2/3 into token-level re-execution — it reintroduces the identical-hardware assumption the founder truth forbids (TAO does not rescue phone-verifies-405B); (c) traps are a statistical audit of the *checkable subset* only, never marketed as closing plausible-but-wrong on open-ended generation; (d) for frontier-hard jobs with no capacity-matched second party, HONE gives cost-bounded **best-effort reasonableness with disclosed residual risk, NOT soundness.**

**Net:** Tier 1 (the objective collapse) is the real, shippable contribution and survives adversarial review — build it first along the collapse-map ordering. The subjective tier holds only after it stops pretending the board "discovers truth" and is reframed as a judge-diverse, escrow-backed, bounded-loss market. The live bond bug and the two dark flags are prerequisites that cost almost nothing and unblock everything downstream.
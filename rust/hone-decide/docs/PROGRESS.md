# hone-decide progress log

Resume notes for whoever picks this up. Newest state at the top of each section.

## Locations
- Code: grouchly `~/hone-decide-wt` (worktree of `~/hone-v21`, branch `decide/open-jev-port`
  from `v2.1-live-identity`). Only `rust/hone-decide` and the workspace member line are ours.
- beastly WSL: `~/decide/` (venv `~/decide/venv`, crate copy `~/decide/crate`, refset,
  seed.json, logs). Models/data: `/mnt/x/ferry/hone-data/hone-models/decide/`
  (`Open-Jev/` clone, `pkg/Open-Jev-{2B,9B}`, `base/{Qwen3.5-2B,Qwen3.5-9B,SmolLM2-*}`,
  `export/`, `data/`, `teacher/`, `students/`).
- grouchly: `~/decide/` (CPU venv for the fp32 Python reference, `xfer/` copies of the
  2B base + package, `export/openjev-2b-f32` for the Rust parity run).
- File transfer from the agent container: Claude `device_commit_files` into
  `X:\hone\decide-xfer\` on beastly, then `scp` to grouchly.

## Phase status
1. Study: done -> `docs/OPEN_JEV_NOTES.md`.
2. Reference run: see below.
3. Rust inference: crate builds on grouchly (candle 0.10.2, CPU); `qwen35` + `llama` backbones.
4. Distillation: scripts in `py/`.
5. Quantise + nebra: `quantize` command, `deploy/hone-decide.service`.

## Log
- 2026-11-02: GPU on beastly has ~7.9 GB free (two llama-server instances, deepseek-r1-8b
  and Qwen3-Coder-30B, hold ~16.6 GB; not ours, not touched). Open-Jev-2B bf16 fits;
  9B bf16 (~15 GB) does not, so the 9B teacher runs 4-bit NF4 (`JEV_LOAD_4BIT=1`) unless
  the card frees up.
- 2026-11-02 18:25: container restart; resumed. Done so far: Open-Jev cloned + studied;
  HF packages Open-Jev-2B/9B, bases Qwen3.5-2B/9B, SmolLM2-135M/360M downloaded to
  `decide/base`, `decide/pkg`; task data generated (`~/decide/data`, 8000/1200/2000 rows,
  copy in `decide/data`); crate builds and its unit tests pass on grouchly; 2B exported
  (merged fp32) to grouchly `~/decide/export/openjev-2b-f32` (320 tensors, 60 LoRA
  modules merged); parity runs started on grouchly: Python fp32 CPU
  (`ref_py_fp32.jsonl`) and Rust (`ref_rust.jsonl`) on `refset.jsonl` (24 rows).
  beastly venv still installing (first torch cu128 install timed out on pypi.nvidia.com;
  second step pulls PyPI torch, driver 617 supports CUDA 13).
- 2026-11-02 18:35: **2B parity PASSED** (grouchly CPU, both fp32): Rust `hone-decide score`
  vs Open-Jev Python `jev_score.py` (JEV_TORCH_DTYPE=float32) on refset (24 rows -> 27
  records, 74 probabilities, choice/noul/score + 4 HONE tasks): max |dp| = 3.59e-6,
  mean 4.7e-7, max |dlogit| 1.9e-5, 0 argmax flips; token ids identical for all 71
  candidate prompts (pyjson + chat template suffix `<think>\n\n</think>\n\n` confirmed).
  Timing on 20 cores: Python 415 s, Rust 439 s for the 27 records (max prompt 607 tokens).
  Files: grouchly `~/decide/ref_py_fp32.jsonl`, `ref_rust.jsonl`, `ref_*tokens/prompts`.
- aarch64 build OK: `CARGO_TARGET_DIR=~/hone-decide-target-arm` with
  `CC_aarch64_unknown_linux_gnu=aarch64-linux-gnu-gcc` and the matching LINKER env; binary
  runs on nebra (9.3 MB).
- Transfer gotcha: `device_commit_files` caches by staged name; use a new file name per transfer.
- 18:55 released-config reference: Open-Jev-2B bf16 on the 3090 (beastly `~/decide/ref_py_bf16_gpu.jsonl`,
  27 records in 9.6 s). bf16-vs-fp32 gap is the same for Python and Rust: max |dp| 0.0125,
  mean 0.0022, 0 argmax flips. So Rust == Python fp32 to 4e-6; both differ from the bf16
  release only by bf16 rounding.
- Llama/student path parity (SmolLM2-135M, untrained head): Rust f32 vs HF fp32 max |dp| 6e-6.
- Quantised matmul fix: candle's CPU q-matmul is per-row vec_dot (12x slower than f32 on
  x86 for prefill). `Lin::fwd` now dequantises a layer to a transient f32 matrix and uses
  gemm when the input has >= 4 rows (`HONE_DECIDE_DEQ_ROWS`). 7x faster.
- nebra first bench (untrained 135M Q8_0, 3 threads, CPUQuota 250%, MemoryMax 250M via
  systemd-run): 200-token ctx 19.9 s (353 tokens), 500-token ctx 34.3 s (647 tokens), peak RSS
  187 / 228 MB. ~18 tok/s on the A53s.
- nebra /tmp is tmpfs (RAM). Never stage files there. Our files live in /home/pi/decide.
  Someone else's 5 x 68 MB hone-node binaries sit in /tmp (340 MB of 454 MB tmpfs), not touched.
- beastly background jobs: launch with Start-Process wsl.exe ... bash ~/decide/bg.sh SCRIPT LOG
  (setsid/nohup jobs die when the launching `wsl -e` exits).
- Teacher selection on 200 test rows: Open-Jev-2B zero-shot on HONE tasks: acc 0.505, ECE 0.236
  (novelty 0.44, routing 0.71, plausibility 0.51, peer 0.38). 2B throughput 5.4 records/s.
- 19:05 Open-Jev-9B bf16 reference (released layout, documented JEV_DEVICE_MAP with
  embeddings/vision/lm_head on CPU) ran once the card was free (llama-servers had exited,
  GPU at 0.5 GB): `ref_py_9b_bf16_gpu.jsonl`, 27 records in 22 s. No Rust 9B parity run
  (merged fp32 9B is ~36 GB, more than grouchly's free RAM); the 9B shares the 2B code path.
- Teacher choice on test200: Open-Jev-9B NF4 acc 0.701 / ECE 0.169 vs 2B bf16 0.505 / 0.236.
  Teacher = 9B NF4 (keeps ~7 GB on the shared card instead of ~16 GB). Labeling all 11.2k
  rows: beastly `~/decide/run_9b2.sh` -> `teacher9b_all.jsonl` (~1.9 records/s).
- Student export check: Rust f32 == HF fp32 for a trained student (max |dp| 2e-7). Python
  bf16-autocast eval differed by up to 1.1 logits, so train_student.py now evaluates and
  fits the temperature in fp32.
- Prefix is fed in 128-token chunks (`HONE_DECIDE_CHUNK`), bit-identical, lower peak memory.
- nebra: kernel cmdline has `cgroup_disable=memory`, so MemoryMax is NOT enforced (for
  hone-node either). Added in-process guards: `--max-rss-mb 240` (exit 1 -> restart) and
  `--min-free-mb 200` (exit 75 -> stays stopped). Both verified on grouchly.
- nebra service installed: /usr/local/bin/hone-decide, /opt/hone-decide/model (currently the
  untrained 135M Q4 placeholder), /etc/systemd/system/hone-decide.service, enabled. RSS 114 MB
  idle; a peer_behavior call (194 tokens) took 14.9 s. hone-node untouched and active.
- nebra Q4 bench (untrained 135M): 200 ctx 20.9 s / RSS 136 MB, 500 ctx 35.0 s / RSS 177 MB.
- 19:35 llama students: candidate suffixes now run as one right-padded batch over the shared
  prefix KV (`forward_batch`), bit-identical to one-by-one. nebra (untrained 135M Q4, 3 threads):
  200 ctx 18.5 s / RSS 127 MB; 500 ctx 37.7 s / RSS 143 MB with 128-token prefill chunks
  (512-token chunks: 33-36 s but RSS 174 MB). Default stays 128 for memory.
- GPU high tier: Rust CUDA build not attempted (qwen35 delta rule runs on CPU slices);
  beastly GPU tier = Open-Jev's own Python stack. Rust qwen35 runs on CPU (grouchly).
- 20:20 Teacher (Open-Jev-9B NF4, its own released T=1.897) on the full HONE test split
  (2000 rows, zero-shot, rule-based ground truth): acc 0.689, ECE 0.190; per task
  novelty 0.718/0.080, routing 0.646/0.280, plausibility 0.772/0.140, peer 0.620/0.273.
- Students training on beastly via `run_students3.sh` (waits for >= 6 GB free VRAM, re-splits
  the teacher labels each time): 135M-kl, 135M-mix (label weight 0.5), 360M-kl, 360M-mix;
  3 epochs, bs 4, lr 1e-4, head lr 1e-3, AdamW8bit. Outputs in beastly `~/decide/students/`.
  The GPU is shared with the deepseek llama-server and ComfyUI; ~1.1 s/step for 135M.
- 21:00 Student smollm2-135m-kl (KL only, 2912 train rows available at start, 3 epochs, T refit
  on cal = 2.80), full 2000-row test, fp32: agreement with teacher 0.793, acc 0.570 (teacher
  0.689), ECE 0.065 (teacher 0.190). Per task agree/acc/ECE: novelty 0.81/0.652/0.125,
  routing 0.856/0.606/0.129, plausibility 0.594/0.470/0.043, peer 0.914/0.550/0.217.
  Rust f32 vs Python student eval on 500 test rows: max |dp| 2.0e-6, 0 flips.
- nebra, trained 135M-kl: Q8_0 200 ctx 17.2 s / RSS 177 MB, 500 ctx 35.7 s / RSS 194 MB;
  Q4 200 ctx 18.4 s / RSS 127 MB, 500 ctx 41.6 s / RSS 143 MB. Deployed: Q4 at
  /opt/hone-decide/model (Q8 would leave < 200 MB MemAvailable at 500-token contexts).
- 135M-mix run was stopped at step 400/4410 (1.8 s/step under GPU contention, ~2.2 h);
  queue is now `run_students4.sh`: 360M-kl (1 epoch) then 135M-mix (2 epochs), starting after
  the teacher finishes.
- Quantisation loss, 135M-kl, Rust, first 500 test rows: f32 acc 0.598 / ECE 0.099;
  Q8_0 acc 0.596 / ECE 0.106 / argmax agreement with f32 0.994 (137 MB GGUF);
  Q4 (Q4_K where cols%256==0, else Q4_0; embeddings Q8_0) acc 0.590 / ECE 0.104 /
  agreement 0.982 (86 MB GGUF). Script: grouchly `~/decide/qeval.sh <student>`.
- 21:24 teacher labels complete (11,200 rows, `teacher9b_all.jsonl`); 360M-kl training started
  on all 8000 train rows (1 epoch, ~0.8 s/step).
- 22:00 smollm2-360m-kl (KL only, 8000 rows, 1 epoch, T 2.97): agreement 0.694, acc 0.481,
  ECE 0.074 (novelty 0.736/0.552/0.070, routing 0.690/0.532/0.109, plausibility
  0.522/0.394/0.037, peer 0.828/0.448/0.132). Under-trained; 3-epoch rerun queued as
  `run_students5.sh` -> `students/smollm2-360m-kl3`.
  360M GGUF: Q8_0 385 MB, Q4 227 MB; Q4 peak RSS 283/307 MB at 200/500 ctx (grouchly):
  does not fit nebra's 250 MB budget. On grouchly (3 threads) Q4: 3.2 s / 7.8 s.
- 22:40 smollm2-135m-mix (0.5 KL to teacher + 0.5 CE on the generator label, 8000 rows,
  2 epochs, T 0.82): agreement 0.719, acc 0.739 (teacher 0.689), ECE 0.022 (teacher 0.190).
  Per task agree/acc/ECE: novelty 0.778/0.632/0.076, routing 0.658/0.836/0.069,
  plausibility 0.690/0.680/0.038, peer 0.748/0.808/0.034. Beats its teacher on routing and
  peer because the rule labels carry information the zero-shot teacher lacks.
- nebra now serves smollm2-135m-mix Q4 (/opt/hone-decide/model). Example: barometer jump
  915 -> 1650 hPa -> "no" p=0.56, 326 tokens, 17.2 s.
- Quantisation loss, Rust, first 500 test rows (acc / ECE / argmax agreement with f32):
  135M-mix f32 0.746/0.062; Q8_0 0.746/0.061/1.000; Q4 0.722/0.055/0.960.
  360M-kl f32 0.502/0.073; Q8_0 0.502/0.074/1.000.
  Q4 costs ~2.4 points on the mix student; Q8 costs nothing measurable. nebra keeps Q4
  because Q8 peaks at ~194 MB and would sit at the 200 MB MemAvailable floor on 500-token
  contexts.
- 360M-kl Q4: acc 0.492 / ECE 0.057 / agreement with f32 0.970.
- 23:50 smollm2-360m-kl3 (KL only, 8000 rows, 3 epochs, T 2.34; beastly
  `students/smollm2-360m-kl3`): agreement 0.908, acc 0.686 (teacher 0.689), ECE 0.049.
  Per task agree/acc/ECE: novelty 0.874/0.744/0.136, routing 0.868/0.638/0.137,
  plausibility 0.920/0.750/0.096, peer 0.970/0.612/0.178. Best pure-distillation student;
  too big for nebra (Q4 RSS ~283-307 MB), fine for grouchly/beastly CPU.
- Caveat: smollm2-135m-kl saw only 2912 train rows (it started before the teacher finished);
  a like-for-like 135M-kl run on all 8000 rows for 3 epochs has not been done.

## State at hand-off
- All beastly GPU jobs are finished; nothing of ours holds the card.
- nebra: `hone-decide.service` active with smollm2-135m-mix Q4; models also in /home/pi/decide.
- Next steps: 135M-kl on full data; 360M-mix; refit the teacher temperature on HONE cal data
  (its released T gives ECE 0.19 here); real (not synthetic) labels for the four tasks.

## Round 2 (2026-11-03): public multiple-choice benchmarks
- `py/prep_mcq.py` -> beastly `~/decide/mcq/` (copy in `decide/mcq`): 8 datasets, 7000 train /
  1557 cal / 2400 test (300 test each). Licenses in `mcq/SOURCES.json`: ARC-E/ARC-C CC-BY-SA-4.0,
  CommonsenseQA MIT, OpenBookQA Apache-2.0, MMLU MIT (6 subjects, test+cal only, never trained),
  HellaSwag MIT, PIQA AFL-3.0, BoolQ CC-BY-SA-3.0 (as `noul`). Where the public test split has
  no labels (CSQA, HellaSwag, PIQA, BoolQ) our test = their validation and cal is carved from
  the tail of train. No benchmark test split is used for training. Rows over 1500 chars dropped.
- Open-Jev-2B bf16 zero-shot on the 2400 test rows: acc 0.720 (chance 0.306), ECE 0.161.
- nebra end-to-end HTTP run 1 (current 135M-mix Q4, 200 questions, 25 per dataset):
  `nebra:~/decide/http_run1_135mix_q4.jsonl`, client `py/nebra_http_test.py` (resumable).
- Open-Jev-9B NF4 zero-shot on the 2400 test rows: acc 0.874, ECE 0.143.
- Existing 135M-mix Q4 (HONE-only) zero-shot via Rust on grouchly: acc 0.307 = chance (0.306).
- nebra HTTP run 1 (135M-mix Q4, 200 q): 200/200 ok, 0 timeouts, p50 10.3 s, p90 15.6 s,
  max 25.3 s, service VmHWM 150 MB, min MemAvailable 439 MB (hone-mem min 466 MB), acc 0.28.
- Students `smollm2-{135m,360m}-gen` (HONE 8000 + MCQ 7000 train rows, 0.5 KL / 0.5 label,
  2 epochs) queued in beastly `run_gen_students.sh`, start after the 9B labels the MCQ train split.
- 360M-kl3 Q8 (HONE-only) zero-shot via Rust: acc 0.287 (chance 0.306).
- Cold 2-epoch gen runs were ~1.5 s/step on the shared card (3 h+ each); replaced with warm
  starts (`--init`): 135M-gen from 135M-mix, 360M-gen from 360M-kl3, 1 epoch over HONE+MCQ
  (15,000 rows), lr 5e-5, 0.5 KL / 0.5 label. Script `run_gen2.sh`, log `gen2.log`.
- Combined score for deployment = mean(HONE test acc, MCQ test acc), both from Rust on the
  deployable quantisation. Current nebra model 135M-mix Q4: 0.722 (HONE, 500 rows) and
  0.307 (MCQ), so 0.515.
- 13:39 smollm2-135m-gen (warm from 135M-mix, 1 epoch HONE+MCQ, T 1.00), Python fp32 on all
  4400 test rows: acc 0.677, ECE 0.030, teacher agreement 0.617. HONE acc
  novelty 0.752 / routing 0.934 / plausibility 0.774 / peer 0.910 (mean 0.843, was 0.739).
  MCQ: ARC-E 0.700, ARC-C 0.463, CSQA 0.547, OBQA 0.557, MMLU 0.363, HellaSwag 0.447,
  PIQA 0.657, BoolQ 0.583 (mean 0.540, chance 0.306). Q8/Q4 Rust eval running on grouchly
  (`ev_135gen_{q4,q8}.jsonl`, evalset = MCQ test + HONE test500). Q4 staged on nebra in
  ~/decide/smollm2-135m-gen-q4. 360M-gen training started 13:38.
- 135M-gen Rust eval (grouchly, `ev_135gen_q4.jsonl`): Q4 MCQ acc 0.520 / ECE 0.018 (2400 rows),
  HONE acc 0.824 / ECE 0.051 (500 rows; same rows as the 135M-mix Q4 baseline at 0.722).
  Combined 0.672 vs 0.515, so it was deployed at 14:18 on nebra. The previous model is
  kept at /opt/hone-decide/model-prev (smollm2-135m-mix-q4). Rollback:
  `sudo systemctl stop hone-decide; sudo mv /opt/hone-decide/model /opt/hone-decide/model-gen;
  sudo mv /opt/hone-decide/model-prev /opt/hone-decide/model; sudo systemctl start hone-decide`.
- nebra was rebooted by another agent with `cgroup_enable=memory cgroup_memory=1`; MemoryMax is
  now enforced (memory.max = 262144000). HTTP run 2 (135M-gen Q4) started 14:18 ->
  `nebra:~/decide/http_run2_135gen_q4.jsonl`.

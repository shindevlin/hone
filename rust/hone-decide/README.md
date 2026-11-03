# hone-decide

A small Rust-native decision engine for HONE. Give it a context, a question and
candidate answers; it returns a calibrated probability per candidate. No text is
generated or parsed.

It is a reimplementation of **[Open-Jev](https://github.com/Zefan-Cai/Open-Jev)**
(MIT, © Open-Jev contributors): the request types, the per-candidate Yes/No prompt,
the scalar head on the last token's final hidden state and the single calibration
temperature all follow Open-Jev. Thanks to the Open-Jev authors for publishing the
method, code and checkpoints. Only Open-Jev was used; nothing comes from TypeSafe's
proprietary Jev. Base models: Qwen3.5 (Apache-2.0, Alibaba Cloud) for the
Open-Jev tiers, SmolLM2 (Apache-2.0, Hugging Face) for the small students.
See `docs/OPEN_JEV_NOTES.md` for how Open-Jev works and what was kept.

## Tiers
| tier | model | where | weights |
|---|---|---|---|
| small | SmolLM2-135M / 360M student, Q8_0 or Q4 GGUF | nebra (Pi CM3+), any CPU | `model-q8_0.gguf` |
| full | Open-Jev-2B / 9B, LoRA merged | grouchly / beastly CPU (Rust); beastly GPU via Open-Jev Python | `model.safetensors` |

A model directory holds `decide.json` (arch + prompt template), `config.json`,
`tokenizer.json`, the weights and `head.safetensors` (`head.weight`, `head.bias`,
`temperature`).

## Use
```
hone-decide serve --model DIR --bind 127.0.0.1:4250 --max-length 1024 --min-free-mb 200
curl -s localhost:4250/decide -d '{"context": {...}, "question": "...", "candidates": ["yes","no","unsure"]}'
# -> {"probs":[...], "candidates":[...], "choice":"yes", "model":"...", "latency_ms":..., "tokens":...}
curl -s localhost:4250/decide -d '{"task": "peer_behavior", "context": {...}}'   # built-in HONE task
```
`candidates` may be a list of names, a list of `{name, description}`, or an object
`{name: description}`. `kind` is `choice` (default), `score` (candidates are ordered
levels) or `noul` (yes/no; probs are `[false, true]`). `GET /tasks` lists the HONE
tasks (`tasks/hone_tasks.json`, extend with `HONE_DECIDE_TASKS=file.json`).
`GET /health` reports RSS and host MemAvailable. With `--min-free-mb N` the process
exits with code 75 when the host's MemAvailable stays under N MB.

Other commands: `score` (batch jsonl, parity runs), `quantize` (safetensors ->
GGUF Q8_0/Q4), `bench` (latency and peak RSS at given context sizes).

## Build
```
cargo build --release -p hone-decide
cargo build --release -p hone-decide --target aarch64-unknown-linux-gnu   # nebra
```

## Python side (`py/`)
`export_openjev.py` (merge LoRA, export head + temperature), `jev_score.py` (reference
and teacher scoring with the real Open-Jev stack), `gen_tasks.py` (HONE task data),
`train_student.py` (distillation), `make_refset.py` (parity set). Training runs in
Python; runtime is this crate. `deploy/hone-decide.service` is the nebra unit.

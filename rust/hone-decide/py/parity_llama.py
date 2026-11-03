"""HF reference scores for a llama student dir (fp32 CPU), same jsonl as `hone-decide score`.
  PYTHONPATH=<Open-Jev> python parity_llama.py STUDENT_DIR rows.jsonl out.jsonl"""
import json, sys
from pathlib import Path
import torch
from safetensors.torch import load_file
from transformers import AutoTokenizer, LlamaConfig, LlamaModel
from jev.api import compile_request, candidate_prompts
from jev.metrics import softmax

d, rows, out = Path(sys.argv[1]), sys.argv[2], sys.argv[3]
cfg = json.loads((d / "decide.json").read_text())
tok = AutoTokenizer.from_pretrained(d)
m = LlamaModel(LlamaConfig.from_pretrained(d)).float().eval()
m.load_state_dict(load_file(str(d / cfg["weights"])), strict=False)
h = load_file(str(d / cfg.get("head", "head.safetensors")))
w, b, T = h["head.weight"][0], h["head.bias"][0], float(h["temperature"][0])
with open(out, "w") as f, torch.no_grad():
    for l in open(rows):
        r = json.loads(l)
        req = r.get("request") or {"state": r["context"], "questions": {"q": {"type": "choice", "instructions": r["question"], "criteria": r["candidates"]}}}
        for rec in compile_request(req["state"], req["questions"]):
            s = []
            for p in candidate_prompts(rec):
                ids = tok(cfg["prompt_prefix"] + p + cfg["prompt_suffix"], return_tensors="pt")["input_ids"]
                s.append(float(m(input_ids=ids).last_hidden_state[0, -1] @ w + b))
            if rec["kind"] == "noul":
                s = [0.0, s[0]]
            rid = r["id"] if "request" not in r else f'{r["id"]}/{rec["id"]}'
            f.write(json.dumps({"id": rid, "logits": s, "probs": softmax(s, T)}) + "\n")

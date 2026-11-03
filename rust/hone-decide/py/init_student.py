"""Untrained student export (backbone as-is, head = E[Yes]-E[No], T=1), for latency/RSS
benchmarks before distillation finishes.  python init_student.py BASE OUT NAME"""
import json, shutil, sys
from pathlib import Path
import torch
from safetensors.torch import load_file, save_file
from transformers import AutoTokenizer

base, out, name = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
out.mkdir(parents=True, exist_ok=True)
sd = load_file(str(base / "model.safetensors"))
sd = {k.replace("model.", "", 1): v.float().contiguous() for k, v in sd.items() if k != "lm_head.weight"}
tok = AutoTokenizer.from_pretrained(base)
E = sd["embed_tokens.weight"]
y, n = tok.encode("Yes", add_special_tokens=False), tok.encode("No", add_special_tokens=False)
w = (E[y[0]] - E[n[0]]).unsqueeze(0) if len(y) == len(n) == 1 else torch.zeros(1, E.shape[1])
save_file(sd, str(out / "model.safetensors"))
save_file({"head.weight": w.contiguous(), "head.bias": torch.zeros(1), "temperature": torch.ones(1)}, str(out / "head.safetensors"))
for f in ("config.json", "tokenizer.json"):
    shutil.copy(base / f, out / f)
(out / "decide.json").write_text(json.dumps({"arch": "llama", "name": name, "weights": "model.safetensors",
    "prompt_prefix": "<|im_start|>user\n", "prompt_suffix": "<|im_end|>\n<|im_start|>assistant\n", "max_length": 2048}, indent=2))
print("yes/no tokens", y, n)

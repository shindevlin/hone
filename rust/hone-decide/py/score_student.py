"""Score rows with a trained student on GPU (fp32, same prompts/head/temperature as Rust).
  PYTHONPATH=<Open-Jev> python score_student.py STUDENT_DIR BASE_DIR rows.jsonl out.jsonl"""
import json, sys
from pathlib import Path
import torch
from safetensors.torch import load_file

sys.path.insert(0, str(Path(__file__).resolve().parent))
from train_student import PREFIX, SUFFIX, Student  # noqa: E402
from jev.api import candidate_prompts, compile_request  # noqa: E402
from jev.metrics import softmax  # noqa: E402

sd, base, rows, out = sys.argv[1:5]
m = Student(base)
m.backbone.load_state_dict(load_file(str(Path(sd) / "model.safetensors")), strict=False)
h = load_file(str(Path(sd) / "head.safetensors"))
with torch.no_grad():
    m.head.weight.copy_(h["head.weight"]); m.head.bias.copy_(h["head.bias"])
T = float(h["temperature"][0])
m = m.cuda().eval()
with open(out, "w") as f, torch.no_grad():
    for l in open(rows):
        r = json.loads(l)
        if "request" in r:
            rec = compile_request(r["request"]["state"], r["request"]["questions"])[0]
        else:
            rec = compile_request(r["context"], {"q": {"type": "choice", "instructions": r["question"], "criteria": r["candidates"]}})[0]
        s = m([PREFIX + p + SUFFIX for p in candidate_prompts(rec)], 2048, amp=False).float().cpu().tolist()
        if rec["kind"] == "noul":
            s = [0.0, s[0]]
        f.write(json.dumps({"id": r["id"], "logits": s, "probs": softmax(s, T)}) + "\n")

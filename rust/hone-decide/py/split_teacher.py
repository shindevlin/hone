"""Split jev_score.py output over data_all.jsonl back into train/cal/test by row id.
  python split_teacher.py OUT_DIR   (run in the dir holding data/ and teacher9b_all.jsonl)"""
import json, sys
from pathlib import Path
split = {}
for s in ("test", "cal", "train"):
    for l in open(f"data/{s}.jsonl"):
        split[json.loads(l)["id"]] = s
out = Path(sys.argv[1]); out.mkdir(exist_ok=True)
fs = {s: open(out / f"{s}.jsonl", "w") for s in ("test", "cal", "train")}
n = {s: 0 for s in fs}
for l in open("teacher9b_all.jsonl"):
    r = json.loads(l); s = split[r["id"]]; fs[s].write(l); n[s] += 1
print(n)

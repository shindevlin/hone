"""Rewrite legacy HONE rows to token-only features (what the node sends to /vote).
  python3 tokenize_rows.py in.jsonl out.jsonl [per_task]"""
import json, sys
from collections import Counter
CPU = {"4x Cortex-A53 1.2GHz": "arm64-4xA53-1.2GHz", "8x Cortex-A76": "arm64-8xA76", "x86_64 20 cores": "x86_64-20c",
       "x86_64 + RTX 3090": "x86_64-RTX3090", "Snapdragon phone": "arm64-snapdragon-phone"}
n = int(sys.argv[3]) if len(sys.argv) > 3 else 10**9
c = Counter()
with open(sys.argv[2], "w") as f:
    for l in open(sys.argv[1]):
        r = json.loads(l)
        if c[r["task"]] >= n:
            continue
        ctx = r["context"]
        if "window" in ctx:
            ctx.pop("window"); ctx["window_epochs"] = 2880
        if isinstance(ctx.get("node"), dict) and ctx["node"].get("cpu") in CPU:
            ctx["node"]["cpu"] = CPU[ctx["node"]["cpu"]]
        c[r["task"]] += 1
        f.write(json.dumps({"task": r["task"], "context": ctx, "label": r["label"]}) + "\n")
print(dict(c))

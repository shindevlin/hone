"""Coverage and accuracy-on-answered at per-task thresholds (thresholds.json) for a pred file.
  python3 sel_eval.py data.jsonl[,more] preds.jsonl thresholds.json"""
import json, sys
R = {}
for p in sys.argv[1].split(","):
    for l in open(p):
        r = json.loads(l)
        keys = list(r["candidates"]) if "candidates" in r else (["false", "true"] if r["request"]["questions"]["q"]["type"] == "noul" else list(r["request"]["questions"]["q"]["criteria"]))
        R[r["id"]] = (r["task"], keys, r["label"])
th = json.load(open(sys.argv[3]))
by = {}
for l in open(sys.argv[2]):
    x = json.loads(l); k = x["id"][:-2] if x["id"].endswith("/q") else x["id"]
    if k not in R:
        continue
    t, keys, lab = R[k]; pr = x["probs"]; c = max(pr)
    d = by.setdefault(t, [0, 0, 0]); d[0] += 1
    if c >= th.get(t, 0.95):
        d[1] += 1; d[2] += keys[pr.index(c)] == lab
out = {t: {"n": n, "threshold": th.get(t, 0.95), "coverage": round(a / n, 3), "acc_answered": round(ok / a, 3) if a else None} for t, (n, a, ok) in sorted(by.items())}
print(json.dumps(out, indent=1))

"""Per-task abstain thresholds from calibration predictions: the lowest confidence t whose
answered set (>= 10 rows) reaches the target accuracy. Tasks that never reach it get 1.01
(always abstain -> deterministic rule / 9B fallback).
  python fit_thresholds.py data_cal.jsonl[,more] preds_cal.jsonl out/thresholds.json [--target 0.95]"""
import json, sys
data, preds, out = sys.argv[1], sys.argv[2], sys.argv[3]
target = float(sys.argv[sys.argv.index("--target") + 1]) if "--target" in sys.argv else 0.95
R = {}
for p in data.split(","):
    for l in open(p):
        r = json.loads(l)
        keys = list(r["candidates"]) if "candidates" in r else (["false", "true"] if r["request"]["questions"]["q"]["type"] == "noul" else list(r["request"]["questions"]["q"]["criteria"]))
        R[r["id"]] = (r["task"], keys, r["label"])
by = {}
for l in open(preds):
    x = json.loads(l); k = x["id"][:-2] if x["id"].endswith("/q") else x["id"]
    if k in R:
        t, keys, lab = R[k]; pr = x["probs"]
        by.setdefault(t, []).append((max(pr), keys[pr.index(max(pr))] == lab))
th, rep = {}, {}
for t, items in sorted(by.items()):
    best = None
    for c in [i / 100 for i in range(30, 100)]:
        sel = [ok for conf, ok in items if conf >= c]
        if len(sel) >= 10 and sum(sel) / len(sel) >= target:
            best = (c, len(sel) / len(items), sum(sel) / len(sel)); break
    th[t] = best[0] if best else 1.01
    rep[t] = {"n": len(items), "threshold": th[t], "coverage": round(best[1], 3) if best else 0.0, "acc": round(best[2], 3) if best else None}
json.dump(th, open(out, "w"), indent=1)
print(json.dumps(rep, indent=1))

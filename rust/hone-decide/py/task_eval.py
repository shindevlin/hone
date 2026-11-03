"""Per-task accuracy / top-label ECE (15 bins) of scored rows vs generator labels,
and optional agreement with a reference scorer.
  python task_eval.py data.jsonl preds.jsonl [ref_preds.jsonl]"""
import json, sys
from collections import defaultdict

rows = {json.loads(l)["id"]: json.loads(l) for l in open(sys.argv[1])}
def ld(p):
    out = {}
    for l in open(p):
        r = json.loads(l)
        out[r["id"][:-2] if r["id"].endswith("/q") else r["id"]] = r
    return out
pred = ld(sys.argv[2])
ref = ld(sys.argv[3]) if len(sys.argv) > 3 else {}
am = lambda v: max(range(len(v)), key=v.__getitem__)
by = defaultdict(list)
for k, p in pred.items():
    if k in rows:
        r = rows[k]
        keys = p.get("answer_keys") or (list(r["candidates"]) if "candidates" in r else
               (["false", "true"] if r["request"]["questions"]["q"]["type"] == "noul"
                else list(r["request"]["questions"]["q"]["criteria"])))
        by[r["task"]].append((p["probs"], keys.index(r["label"]), ref.get(k, {}).get("probs")))
by["ALL"] = [x for t in list(by) for x in by[t]]
out = {}
for t, items in by.items():
    bins = [[0, 0.0, 0.0] for _ in range(15)]
    for pr, lab, _ in items:
        c = max(pr); b = bins[min(int(c * 15), 14)]; b[0] += 1; b[1] += c; b[2] += am(pr) == lab
    o = {"n": len(items), "acc": round(sum(am(p) == l for p, l, _ in items) / len(items), 4),
         "ece": round(sum(abs(b[1] - b[2]) for b in bins) / len(items), 4)}
    agr = [am(p) == am(q) for p, _, q in items if q]
    if agr:
        o["agree"] = round(sum(agr) / len(agr), 4)
    o["chance"] = round(sum(1 / len(p) for p, _, _ in items) / len(items), 3)
    out[t] = o
print(json.dumps(out))

"""Compare two scored jsonl files (by id): max/mean abs prob diff, argmax flips.
  python compare.py a.jsonl b.jsonl [--tokens py_prompts.jsonl rust_tokens.jsonl]"""
import json, sys

def load(p):
    return {json.loads(l)["id"]: json.loads(l) for l in open(p) if l.strip()}

def ids(p):
    out = {}
    for l in open(p):
        r = json.loads(l)
        out.setdefault(r["id"], []).append(r["ids"])
    return out

a, b = load(sys.argv[1]), load(sys.argv[2])
common = [k for k in a if k in b]
diffs, flips, ldiff = [], 0, []
for k in common:
    pa, pb = a[k]["probs"], b[k]["probs"]
    diffs += [abs(x - y) for x, y in zip(pa, pb)]
    ldiff += [abs(x - y) for x, y in zip(a[k]["logits"], b[k]["logits"])]
    flips += max(range(len(pa)), key=pa.__getitem__) != max(range(len(pb)), key=pb.__getitem__)
res = {"records": len(common), "probs": len(diffs), "max_abs_prob_diff": max(diffs) if diffs else None,
       "mean_abs_prob_diff": sum(diffs) / max(len(diffs), 1), "max_abs_logit_diff": max(ldiff) if ldiff else None,
       "argmax_flips": flips}
if "--tokens" in sys.argv:
    i = sys.argv.index("--tokens")
    ta, tb = ids(sys.argv[i + 1]), ids(sys.argv[i + 2])
    res["token_seqs_compared"] = sum(len(ta[k]) for k in ta if k in tb)
    res["token_mismatch_records"] = [k for k in ta if k in tb and ta[k] != tb[k]]
print(json.dumps(res, indent=1))

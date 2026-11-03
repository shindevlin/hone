"""Selective accuracy / confidence map.
  python conf_map.py --data a.jsonl,b.jsonl --pred NAME=preds.jsonl [--pred ...] --out map.json
Confidence = max probability. For each model x group (task, kind, n-options, length bucket)
reports coverage and accuracy at thresholds, and the lowest threshold whose answered set
reaches >= 95% accuracy (with coverage). A group is "high-trust" if that coverage >= 30%.
"""
import argparse, json
from collections import defaultdict

TH = [0.0, 0.5, 0.6, 0.7, 0.8, 0.9, 0.95]
HONE = {"sensor_plausibility", "reading_novelty", "job_routing", "peer_behavior"}


def rows(paths):
    out = {}
    for p in paths:
        for l in open(p):
            r = json.loads(l)
            if "request" in r:
                q = r["request"]["questions"]["q"]
                keys = ["false", "true"] if q["type"] == "noul" else list(q["criteria"])
                kind, size = q["type"], len(json.dumps(r["request"], ensure_ascii=False))
            else:
                keys, kind = list(r["candidates"]), "choice"
                size = len(json.dumps(r["context"], ensure_ascii=False))
            out[r["id"]] = {"task": r["task"], "kind": kind, "keys": keys, "label": r["label"], "chars": size}
    return out


def preds(p):
    out = {}
    for l in open(p):
        r = json.loads(l)
        out[r["id"][:-2] if r["id"].endswith("/q") else r["id"]] = r["probs"]
    return out


def lenbucket(c):
    return "<400c" if c < 400 else "400-800c" if c < 800 else "800-1200c" if c < 1200 else ">=1200c"


def stats(items):
    res = {}
    n = len(items)
    for t in TH:
        sel = [ok for conf, ok in items if conf >= t]
        res[str(t)] = {"cov": round(len(sel) / n, 3) if n else 0, "acc": round(sum(sel) / len(sel), 3) if sel else None}
    # lowest threshold (fine grid) reaching >= 95% on answered set
    best = None
    for t in [x / 100 for x in range(30, 100)]:
        sel = [ok for conf, ok in items if conf >= t]
        if len(sel) >= 10 and sum(sel) / len(sel) >= 0.95:
            best = {"t": t, "cov": round(len(sel) / n, 3), "acc": round(sum(sel) / len(sel), 3)}
            break
    return {"n": n, "th": res, "t95": best, "high_trust": bool(best and best["cov"] >= 0.30)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--data", required=True)
    ap.add_argument("--pred", action="append", required=True)
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    R = rows(a.data.split(","))
    report = {}
    for spec in a.pred:
        name, path = spec.split("=", 1)
        P = preds(path)
        groups = defaultdict(list)
        for k, pr in P.items():
            if k not in R:
                continue
            r = R[k]
            conf = max(pr)
            ok = r["keys"][max(range(len(pr)), key=pr.__getitem__)] == r["label"]
            fam = "HONE" if r["task"] in HONE else "MCQ"
            for g in (f"all", f"family:{fam}", f"task:{r['task']}", f"kind:{r['kind']}",
                      f"options:{len(r['keys'])}", f"len:{lenbucket(r['chars'])}", f"{fam}-len:{lenbucket(r['chars'])}"):
                groups[g].append((conf, ok))
        report[name] = {g: stats(v) for g, v in sorted(groups.items())}
    json.dump(report, open(a.out, "w"), indent=1)
    for name, gs in report.items():
        print(f"== {name}")
        print(f"{'group':34} {'n':>5} " + " ".join(f"c@{t}/a" for t in ("0.7", "0.9", "0.95")) + "   t95(cov)")
        for g, s in gs.items():
            cells = " ".join(f"{s['th'][t]['cov']:.2f}/{(s['th'][t]['acc'] or 0):.2f}" for t in ("0.7", "0.9", "0.95"))
            t95 = f"{s['t95']['t']:.2f}({s['t95']['cov']:.2f})" if s["t95"] else "-"
            print(f"{g:34} {s['n']:>5} {cells}   {t95}{'  *' if s['high_trust'] else ''}")


if __name__ == "__main__":
    main()

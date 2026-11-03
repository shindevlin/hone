"""Jury benchmark over HTTP on a node: vote labeled HONE rows in small batches, one test
epoch per batch, seal, verify every reveal, record latency/memory/coverage/accuracy.
Stops if MemAvailable < 200 MB or hone-node restarts. Run on the node itself.
  python3 jury_bench.py rows.jsonl out.json [--url http://127.0.0.1:4250] [--batch 4] [--epoch0 100]
rows.jsonl: {task, context, label} (token-only features)."""
import hashlib, json, subprocess, sys, time, urllib.request, urllib.error
sys.path.insert(0, __file__.rsplit("/", 1)[0])
from jury_verify import verify

args = sys.argv[1:]
opt = {"--url": "http://127.0.0.1:4250", "--batch": "4", "--epoch0": "100"}
pos = []
i = 0
while i < len(args):
    if args[i] in opt:
        opt[args[i]] = args[i + 1]; i += 2
    else:
        pos.append(args[i]); i += 1
rows_p, out_p = pos
url, B, E = opt["--url"], int(opt["--batch"]), int(opt["--epoch0"])


def call(m, p, body=None, timeout=600):
    req = urllib.request.Request(url + p, method=m, data=None if body is None else json.dumps(body).encode(),
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read())
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}")


def avail():
    for l in open("/proc/meminfo"):
        if l.startswith("MemAvailable:"):
            return int(l.split()[1]) // 1024


def node():
    o = subprocess.run(["systemctl", "show", "hone-node", "-p", "NRestarts", "-p", "MainPID", "--value"], capture_output=True, text=True).stdout.split()
    return tuple(o)


rows = [json.loads(l) for l in open(rows_p)]
n0, recs, per_item, min_av, max_rss, stop = node(), [], [], 10**9, 0, None
for k in range(0, len(rows), B):
    a = avail(); min_av = min(min_av, a)
    if a < 200 or node() != n0:
        stop = f"guard: avail={a} node={node()} was {n0}"; break
    batch = rows[k:k + B]; ep = E + k // B
    items = [{"entry_hash": hashlib.sha256((json.dumps(r["context"], sort_keys=True) + str(k + j)).encode()).hexdigest(),
              "task": r["task"], "features": r["context"]} for j, r in enumerate(batch)]
    t0 = time.time(); st, out = call("POST", "/vote", {"epoch": ep, "items": items}); wall = time.time() - t0
    if st != 200:
        recs.append({"epoch": ep, "status": st, "error": out}); continue
    per_item.append(wall * 1000 / len(items))
    _, c = call("POST", f"/commit/{ep}")
    byh = {v["entry_hash"]: v for v in out["votes"]}
    for it, r in zip(items, batch):
        v = byh.get(it["entry_hash"])
        rec = {"task": r["task"], "label": r["label"], "epoch": ep}
        if v is None:
            rec["error"] = next((e["error"] for e in out["errors"] if e["entry_hash"] == it["entry_hash"]), "missing")
        else:
            _, rev = call("GET", f"/reveal/{ep}/{it['entry_hash']}")
            rec.update(option=v["option"], conf_q8=v["conf_q8"], t95_q8=v["t95_q8"], verified=verify(rev, c["root"])[0])
        recs.append(rec)
    _, h = call("GET", "/health"); max_rss = max(max_rss, h.get("rss_mb") or 0); min_av = min(min_av, h.get("mem_available_mb") or 10**9)
    print(ep, f"{wall:.1f}s", [x.get("option") for x in recs[-len(batch):]], flush=True)
_, cap = call("GET", "/capacity")
lat = sorted(per_item)
q = lambda f: round(lat[min(len(lat) - 1, int(f * len(lat)))]) if lat else None
voted = [r for r in recs if "option" in r]
ans = [r for r in voted if r["option"] != "abstain"]
by = {}
for r in voted:
    d = by.setdefault(r["task"], {"n": 0, "answered": 0, "correct": 0})
    d["n"] += 1
    if r["option"] != "abstain":
        d["answered"] += 1; d["correct"] += r["option"] == r["label"]
summary = {"rows": len(rows), "voted": len(voted), "errors": sum("error" in r for r in recs), "stopped": stop,
           "ms_per_item_p50": q(0.5), "ms_per_item_p90": q(0.9), "ms_per_item_max": round(lat[-1]) if lat else None,
           "coverage": round(len(ans) / len(voted), 3) if voted else None,
           "accuracy_answered": round(sum(r["option"] == r["label"] for r in ans) / len(ans), 3) if ans else None,
           "reveals_verified": f"{sum(r.get('verified', False) for r in voted)}/{len(voted)}",
           "per_task": {t: {**d, "coverage": round(d["answered"] / d["n"], 3), "acc": round(d["correct"] / d["answered"], 3) if d["answered"] else None} for t, d in by.items()},
           "max_rss_mb": max_rss, "min_avail_mb": min_av, "capacity": cap, "hone_node_unchanged": node() == n0}
json.dump({"summary": summary, "records": recs}, open(out_p, "w"), indent=1)
print(json.dumps(summary, indent=1))

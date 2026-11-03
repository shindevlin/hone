"""End-to-end test of a running hone-decide service over HTTP (run on the node itself).

  python3 nebra_http_test.py batch.jsonl out.jsonl [--url http://127.0.0.1:4250] [--timeout 180] [--min-free-mb 200]

batch.jsonl rows: {id, task, label, body} where body is the POST /decide payload.
Resumable: ids already in out.jsonl are skipped (survives service restarts / reboots).
Stops if host MemAvailable < --min-free-mb. Prints a summary at the end.
"""
import json, sys, time, urllib.request, urllib.error

args = sys.argv[1:]
opt = {"--url": "http://127.0.0.1:4250", "--timeout": "180", "--min-free-mb": "200"}
pos = []
i = 0
while i < len(args):
    if args[i] in opt:
        opt[args[i]] = args[i + 1]; i += 2
    else:
        pos.append(args[i]); i += 1
batch, outp = pos
url, timeout, min_free = opt["--url"], float(opt["--timeout"]), int(opt["--min-free-mb"])


def avail_mb():
    for l in open("/proc/meminfo"):
        if l.startswith("MemAvailable:"):
            return int(l.split()[1]) // 1024
    return -1


def get(path):
    with urllib.request.urlopen(url + path, timeout=10) as r:
        return json.loads(r.read())


done = set()
try:
    done = {json.loads(l)["id"] for l in open(outp) if l.strip()}
except FileNotFoundError:
    pass
rows = [json.loads(l) for l in open(batch) if l.strip()]
out = open(outp, "a")
for r in rows:
    if r["id"] in done:
        continue
    a = avail_mb()
    if a < min_free:
        print(f"STOP: MemAvailable {a} MB < {min_free} MB", flush=True)
        break
    rec = {"id": r["id"], "task": r["task"], "label": r["label"], "avail_mb_before": a}
    for attempt in range(3):
        t0 = time.time()
        try:
            req = urllib.request.Request(url + "/decide", data=json.dumps(r["body"]).encode(),
                                         headers={"Content-Type": "application/json"})
            with urllib.request.urlopen(req, timeout=timeout) as resp:
                res = json.loads(resp.read())
            rec.update(ok=True, wall_ms=round((time.time() - t0) * 1000, 1), latency_ms=res["latency_ms"],
                       tokens=res["tokens"], model=res["model"], probs=res["probs"],
                       candidates=res["candidates"], choice=res["choice"],
                       confidence=res.get("confidence"), decision=res.get("decision"), threshold=res.get("threshold"))
            break
        except urllib.error.HTTPError as e:
            rec.update(ok=False, error=f"HTTP {e.code}: {e.read()[:200]!r}", wall_ms=round((time.time() - t0) * 1000, 1))
            break
        except Exception as e:  # connection refused (restart), timeout
            rec.update(ok=False, error=f"{type(e).__name__}: {e}", wall_ms=round((time.time() - t0) * 1000, 1))
            if "timed out" in str(e):
                break
            time.sleep(30)  # service restarting: wait and retry
    try:
        h = get("/health"); rec["rss_mb"] = h.get("rss_mb"); rec["avail_mb_after"] = h.get("mem_available_mb")
    except Exception:
        pass
    out.write(json.dumps(rec) + "\n"); out.flush()
    print(rec["id"], rec.get("choice"), rec.get("wall_ms"), rec.get("error", ""), flush=True)

# summary
res = [json.loads(l) for l in open(outp) if l.strip()]
ok = [x for x in res if x.get("ok")]
lat = sorted(x["wall_ms"] for x in ok)
pct = lambda q: lat[min(len(lat) - 1, int(q * len(lat)))] if lat else None
acc = sum(x["choice"] == x["label"] for x in ok) / len(ok) if ok else None
by = {}
for x in ok:
    by.setdefault(x["task"], []).append(x["choice"] == x["label"])
print(json.dumps({"n": len(res), "ok": len(ok), "failures": len(res) - len(ok),
                  "p50_ms": pct(0.5), "p90_ms": pct(0.9), "max_ms": lat[-1] if lat else None,
                  "accuracy": acc, "per_task": {k: round(sum(v) / len(v), 3) for k, v in by.items()},
                  "answered": sum(x.get("decision") != "abstain" for x in ok),
                  "accuracy_answered": (lambda a: sum(x["choice"] == x["label"] for x in a) / len(a) if a else None)(
                      [x for x in ok if x.get("decision") != "abstain"]),
                  "max_rss_mb_seen": max((x.get("rss_mb") or 0) for x in res),
                  "min_avail_mb_seen": min(min(x.get("avail_mb_before", 9999), x.get("avail_mb_after") or 9999) for x in res)}))

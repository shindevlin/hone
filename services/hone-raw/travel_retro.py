#!/usr/bin/env python3
"""Retroactive travel-group placement of stored Flipper frames (Josh 2026-11-14).

For an owner and a time range: every Flipper commit frame in the indexer
(/api/flipper/frames) is classed
  A  heard live by a HONE phone (its cell came from that phone at hearing time)
  B  travel_group: an owner's phone raw window at the same time, plus a radio co-location match
     (needs the Flipper's own radio environment, which commit frames do not carry yet)
  C  inferred: an owner's phone area at that window, no proof
  -  nothing to place it with
Phone tracks stay on the phones; here the phone side comes from the public raw index (area
cell per window). Prints a JSON summary; writes nothing.

usage: travel_retro.py OWNER FROM_ISO TO_ISO [--base https://honemesh.net]
"""
import json, sys, urllib.request, datetime as dt

def get(url):
    with urllib.request.urlopen(url, timeout=60) as r:
        return json.loads(r.read())

def main():
    owner, t0s, t1s = sys.argv[1:4]
    base = sys.argv[sys.argv.index("--base") + 1] if "--base" in sys.argv else "https://honemesh.net"
    t0 = int(dt.datetime.fromisoformat(t0s).replace(tzinfo=dt.timezone.utc).timestamp())
    t1 = int(dt.datetime.fromisoformat(t1s).replace(tzinfo=dt.timezone.utc).timestamp())
    frames = [f for f in get(f"{base}/api/flipper/frames?owner={owner}&since={t0 // 60}&limit=20000") if t0 <= f["t_min"] * 60 <= t1]
    raw = get(f"{base}/api/raw/index?account={owner}&from={t0 // 900}&to={t1 // 900}&limit=2000")
    phone_by_win = {}
    for r in raw:
        if r.get("kind") == "phone" and r.get("cell"):
            phone_by_win.setdefault(r["window"], set()).add(r["cell"])
    out = {"owner": owner, "from": t0s, "to": t1s, "frames": len(frames), "A_heard": 0, "B_travel_group": 0, "C_inferred": 0,
           "unplaced": 0, "phone_raw_windows": len(phone_by_win), "examples": []}
    for f in frames:
        w = f["t_min"] * 60 // 900
        if f.get("cells"):
            out["A_heard"] += 1; tier, cell = "A", f["cells"][0]
        elif phone_by_win.get(w):
            out["C_inferred"] += 1; tier, cell = "C", sorted(phone_by_win[w])[0]   # no Flipper radio env: no proof possible
        else:
            out["unplaced"] += 1; tier, cell = "-", None
        if len(out["examples"]) < 5:
            out["examples"].append({"sensor_id": f["sensor_id"], "t_min": f["t_min"], "tier": tier, "cell": cell})
    print(json.dumps(out, indent=1))

if __name__ == "__main__":
    main()

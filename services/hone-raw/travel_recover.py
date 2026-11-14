#!/usr/bin/env python3
"""Recover a Flipper's stored frames (SD-card export) into hone.raw late data, marked recovered.

  travel_recover.py --frames F.bin [F2.bin ...] --pub identity.pub --day 2026-11-11 --owner josh
                    [--coverage cov.json] [--phone-env phone_env.json] [--out DIR] [--key KEYFILE --upload URL]

1. Parse the H2 frames (len byte + 32-byte header + body + 64-byte ed25519 signature), verify every signature
   against identity.pub, check the device id, dedupe by counter across files and check the counter chain
   (strictly increasing; gaps reported). Frames that fail any check are dropped and counted.
2. Decode: 1 subghz (Hz, peak, avg), 5 heartbeat (battery), 6 ir, 9 window summary, 11 Wi-Fi scan
   (22-byte header, then n x [8-byte token, rssi, channel]).
3. Place each 15-min window:
   B travel_group  phone track position + radio co-location proof (Wi-Fi Jaccard / BLE / sub-GHz) + home anchor.
                   Needs the phone's own radio environment for that window (--phone-env); none exists for
                   phones on vc26 or older, so B is impossible for their data.
   C inferred      the owner's phone reported coverage from exactly one cell in that window (--coverage,
                   rows exported from the indexer), no co-location proof.
   -               no phone position for that window: area src "none".
4. One hone.raw blob per window: hdr (recovered, late, frame counter range, sha256 of the original signed frames)
   plus obs@2 records with full provenance (why "recovered"). With --upload, each blob is signed by --key under
   OWNER and posted to the normal ingest (/api/raw/upload). Without it, a dry run writes the blobs to --out.
"""
import argparse, collections, datetime as dt, gzip, hashlib, json, os, struct, sys, time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hone_rawfmt as F

NAMES = {1: "subghz", 2: "rfid", 3: "nfc", 4: "ibutton", 5: "heartbeat", 6: "ir", 7: "wifi", 9: "summary", 11: "wifi_scan"}


def read_frames(path):
    d = open(path, "rb").read(); i = 0
    while i < len(d):
        n = d[i]; f = d[i + 1:i + 1 + n]; i += 1 + n
        if len(f) == n and len(f) >= 96 and f[:2] == b"H2":
            yield f


def parse(paths, pub, day0, day1):
    from nacl.signing import VerifyKey
    vk = VerifyKey(pub); dev = pub[:8]
    by_ctr, bad_sig, bad_dev = {}, 0, 0
    for p in paths:
        for f in read_frames(p):
            typ, blen = f[2], f[3]
            ctr, rtc, up = struct.unpack("<III", f[12:24])
            if f[4:12] != dev: bad_dev += 1; continue
            try: vk.verify(f[:32 + blen], f[32 + blen:32 + blen + 64])
            except Exception: bad_sig += 1; continue
            if day0 <= rtc < day1: by_ctr.setdefault(ctr, f)
    ctrs = sorted(by_ctr); frames = [by_ctr[c] for c in ctrs]
    rtcs = [struct.unpack("<I", f[16:20])[0] for f in frames]
    gaps = [(a, b) for a, b in zip(ctrs, ctrs[1:]) if b != a + 1]
    chain = {"frames": len(frames), "bad_signature": bad_sig, "wrong_device": bad_dev,
             "counter_first": ctrs[0] if ctrs else None, "counter_last": ctrs[-1] if ctrs else None,
             "counter_gaps": len(gaps), "missing_counters": sum(b - a - 1 for a, b in gaps),
             "rtc_monotonic": all(b >= a for a, b in zip(rtcs, rtcs[1:]))}
    return frames, chain


def decode(f):
    typ, blen = f[2], f[3]; b = f[32:32 + blen]
    ctr, rtc, up = struct.unpack("<III", f[12:24])
    out = {"type": NAMES.get(typ, str(typ)), "ctr": ctr, "t": rtc}
    if typ == 1 and blen >= 8:
        hz, peak, avg, _ = struct.unpack("<IbbH", b[:8]); out.update(hz=hz, peak=peak, avg=avg)
    elif typ == 5 and blen >= 17:
        out.update(battery=b[0])
    elif typ == 6 and blen >= 12:
        pr, rep, addr, cmd, raw = struct.unpack("<bBIIH", b[:12]); out.update(proto=pr)
    elif typ == 11 and blen >= 22:
        n = b[21]
        out["aps"] = [(b[22 + 10 * k:30 + 10 * k].hex(), struct.unpack("b", b[30 + 10 * k:31 + 10 * k])[0], b[31 + 10 * k])
                      for k in range(n) if 32 + 10 * k <= len(b)]
    return out


def place(win, coverage, phone_env, fl_env):
    """(tier, area) for one window."""
    cells = collections.Counter(r["cell"] for r in coverage.get(win, []))
    env = phone_env.get(win)
    if env is not None and len(cells) == 1:
        m = F.colocate(fl_env, env)
        if m.get("match"):
            return "B", {"cell": next(iter(cells)), "res": 7, "src": "travel_group", "inferred": False, "proof": m.get("proof")}
    if len(cells) == 1:
        return "C", {"cell": next(iter(cells)), "res": 7, "src": "phone_window", "inferred": True}
    return "-", {"cell": None, "res": None, "src": "none", "inferred": True}


def to_res7(cell):
    try:
        import h3
        return h3.cell_to_parent(cell, 7) if h3.get_resolution(cell) > 7 else cell
    except Exception:
        return cell


def main():
    a = argparse.ArgumentParser()
    a.add_argument("--frames", nargs="+", required=True); a.add_argument("--pub", required=True)
    a.add_argument("--day", required=True); a.add_argument("--owner", required=True)
    a.add_argument("--coverage"); a.add_argument("--phone-env"); a.add_argument("--out", default="recovered")
    a.add_argument("--key"); a.add_argument("--upload")
    o = a.parse_args()
    pub = open(o.pub, "rb").read(); devid = pub[:8].hex()
    d0 = int(dt.datetime.fromisoformat(o.day).replace(tzinfo=dt.timezone.utc).timestamp()); d1 = d0 + 86400
    frames, chain = parse(o.frames, pub, d0, d1)
    coverage = collections.defaultdict(list)
    if o.coverage:
        for r in json.load(open(o.coverage)).get("coverage", []):
            coverage[int(r["window"])].append({"cell": to_res7(r["cell"]), "device": r["device"]})
    phone_env = {int(k): v for k, v in json.load(open(o.phone_env)).items()} if o.phone_env else {}

    wins = collections.defaultdict(list)
    for f in frames:
        wins[struct.unpack("<I", f[16:20])[0] // 900].append(f)
    os.makedirs(o.out, exist_ok=True)
    sk = None
    if o.key:
        from nacl.signing import SigningKey
        sk = SigningKey(bytes.fromhex(open(o.key).read().strip()))
        import hone_raw_client as C
    summary = {"device": devid, "day": o.day, "chain": chain, "windows": len(wins), "tiers": collections.Counter(),
               "frames_by_tier": collections.Counter(), "cells": collections.Counter(), "types": collections.Counter(),
               "uploaded": 0, "upload_errors": []}
    for w in sorted(wins):
        fs = wins[w]; dec = [decode(f) for f in fs]
        toks = {ap[0] for x in dec if x["type"] == "wifi_scan" for ap in x.get("aps", [])}
        fl_env = {"wifi": sorted(toks), "ble": 0, "subghz": [x["t"] for x in dec if x["type"] == "subghz" and x.get("peak", -128) > -60]}
        tier, area = place(w, coverage, phone_env, fl_env)
        loc = area["src"] if area["cell"] else "none"
        src_sha = hashlib.sha256(b"".join(bytes([len(f)]) + f for f in fs)).hexdigest()
        hdr = {"s": F.SCHEMA_HDR, "t": F.rfc3339(w * 900), "dev": devid, "owner": o.owner, "cls": "flipper", "win": w,
               "area": area, "tier": tier, "recovered": True, "late": True, "source": "sd_export",
               "frames": {"count": len(fs), "ctr_first": dec[0]["ctr"], "ctr_last": dec[-1]["ctr"], "sha256": src_sha,
                          "pubkey": pub.hex(), "format": "len8|H2 hdr32|body|ed25519 sig64"}}
        recs = [hdr]

        def obs(t, k, v, ot, sen, **extra):
            recs.append({"s": F.SCHEMA_OBS, "t": F.rfc3339(t), "k": k, "v": v, "dev": devid, "cls": "flipper", "sen": sen,
                         "why": "recovered", "loc": loc, "obs_type": ot, "cap": sen, **extra})
        for x in dec:
            summary["types"][x["type"]] += 1
            if x["type"] == "subghz" and "hz" in x:
                obs(x["t"], f"subghz:{x['hz']}", x["peak"], "subghz", "subghz", avg=x["avg"], ctr=x["ctr"])
            elif x["type"] == "wifi_scan":
                for tok, rssi, ch in x.get("aps", []):
                    obs(x["t"], f"wifi:{tok}", rssi, "wifi_env", "wifi_board", ch=ch, ctr=x["ctr"])
            elif x["type"] == "heartbeat" and "battery" in x:
                obs(x["t"], "dev.battery_pct", x["battery"], "device", "battery", ctr=x["ctr"])
        body = gzip.compress(F.to_jsonl(recs), compresslevel=9, mtime=0)
        if F.check_prov(recs) is not None:
            raise SystemExit(f"window {w}: provenance check failed")
        open(os.path.join(o.out, f"{devid}-{w}.jsonl.gz"), "wb").write(body)
        summary["tiers"][tier] += 1; summary["frames_by_tier"][tier] += len(fs)
        if area["cell"]: summary["cells"][area["cell"]] += len(fs)
        if sk and o.upload:
            mb, sig = C.meta(sk, o.owner, devid, w, area["cell"], body, "flipper_recovered")
            st, resp = C.upload(o.upload, mb, sig, body)
            if st == 200: summary["uploaded"] += 1
            else: summary["upload_errors"].append([w, st, str(resp)[:120]])
            time.sleep(0.2)
    rt = [struct.unpack("<I", f[16:20])[0] for f in frames]
    summary["span"] = [F.rfc3339(min(rt)), F.rfc3339(max(rt))] if rt else None
    print(json.dumps(summary, indent=1, default=dict))


if __name__ == "__main__":
    main()

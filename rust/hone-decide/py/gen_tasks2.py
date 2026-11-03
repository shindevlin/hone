"""Round-3 HONE decision tasks shaped for the confident zone (CONFIDENCE_MAP.md):
3 named options, short JSON context with pre-computed comparisons, deterministic labels.

  coverage_usefulness   CoverageReport vs the cell's state this epoch (mirrors chain.rs
                        CoverageReport apply: 0.001 deg grid, 200/epoch cap, 3-reporter
                        corroboration, dead spot < -110 dBm) -> new / adds / repeat
  entry_screen          pending entry vs submitter's recent nonce, hashes, rate, clock skew
                        -> accept / duplicate / spam
  sensor_plausibility_v2  v1 plausibility with derived features (median, delta, max step,
                        in-range) so the model compares instead of computing

Rows look like gen_tasks.py rows; a teacher file with smoothed one-hot probs is written
alongside (labels are deterministic, no 9B needed).
  python gen_tasks2.py --seed-json seed.json --out data3/ --teacher-out data3_t/
"""
import argparse, hashlib, json, random, statistics, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gen_tasks as g1  # noqa: E402

TASKS = json.loads((Path(__file__).resolve().parent.parent / "tasks" / "hone_tasks.json").read_text())["tasks"]
CAP, CORR_MIN, DEAD_DBM, CHANGE_DB = 200, 3, -110, 10
CARRIERS = ["706-01", "706-02", "706-03", "310-260", "334-020", "234-15"]
TECH = ["LTE", "NR", "UMTS", "GSM"]


def h(rng):
    return hashlib.sha256(str(rng.random()).encode()).hexdigest()[:16]


def gen_coverage(rng, seed, label):
    accts = seed.get("accounts") or ["josh", "shindevlin", "natoshisakamoto"]
    for _ in range(1000):
        me = rng.choice(accts + [f"user{rng.randrange(100, 999)}"])
        epoch = 352000 + rng.randrange(0, 4000)
        lat, lon = round(13.67 + rng.uniform(-0.2, 0.2), 5), round(-89.25 + rng.uniform(-0.2, 0.2), 5)
        cell = f"{int(lat * 1000)}:{int(lon * 1000)}"
        carrier = rng.choice(CARRIERS)
        dbm = rng.choice([None] + [rng.randrange(-125, -55)] * 6)
        n_prior = rng.choice([0, 0, 1, 2, 3, 4, 6])
        others = rng.sample([a for a in accts + [f"user{i}" for i in range(20)] if a != me], min(n_prior, 10))
        prior = list(others)
        if n_prior and rng.random() < 0.3:
            prior.insert(rng.randrange(len(prior) + 1), me)
        prior = prior[:CORR_MIN + 2]
        dbms = [rng.randrange(-120, -60) for _ in prior]
        med = int(statistics.median(dbms)) if dbms else None
        mine = rng.choice([0, 1, 3, 20, 150, 199, 200]) if rng.random() < 0.5 else rng.randrange(0, 40)
        in_cell = me in prior
        distinct = len(set(prior))
        corroborated = distinct >= CORR_MIN
        delta = None if (dbm is None or med is None) else dbm - med
        if mine >= CAP or in_cell:
            lab = "repeat"
        elif not prior:
            lab = "new"
        elif not corroborated or (delta is not None and abs(delta) > CHANGE_DB):
            lab = "adds"
        else:
            lab = "repeat"
        if lab != label:
            continue
        return {
            "report": {"reporter": me, "epoch": epoch, "lat": lat, "lon": lon, "cell": cell, "carrier": carrier,
                       "technology": rng.choice(TECH), "signal_dbm": dbm,
                       "dead_spot": dbm is None or dbm < DEAD_DBM},
            "cell_this_epoch": {"reporters": prior, "distinct_reporters": distinct, "median_dbm": med,
                                "corroborated": corroborated},
            "derived": {"reporter_already_in_cell": in_cell, "signal_delta_vs_median_db": delta,
                        "reporter_reports_this_epoch": mine},
            "rules": {"reports_cap_per_epoch": CAP, "corroboration_min_reporters": CORR_MIN,
                      "material_signal_change_db": CHANGE_DB},
        }
    raise RuntimeError("coverage generator failed")


ENTRY_TYPES = ["SensorCommit", "CoverageReport", "TrackerSightingCommit", "Transfer", "BlobStore"]


def gen_entry(rng, seed, label):
    accts = seed.get("accounts") or ["josh", "shindevlin", "natoshisakamoto"]
    for _ in range(1000):
        who = rng.choice(accts)
        last_nonce = rng.randrange(1, 5000)
        recent = [h(rng) for _ in range(rng.randrange(2, 8))]
        nonce = last_nonce + rng.choice([1, 1, 1, 2, 5, 0, -3])
        dh = rng.choice(recent) if rng.random() < 0.25 else h(rng)
        rate, cap = rng.randrange(0, 60), rng.choice([20, 50, 200])
        if rng.random() < 0.2:
            rate = cap + rng.randrange(0, 50)
        skew = int(rng.gauss(0, 20000)) if rng.random() < 0.85 else rng.choice([-1, 1]) * rng.randrange(120000, 900000)
        size = rng.choice([0] + [rng.randrange(40, 4000)] * 9)
        dup = dh in recent or nonce <= last_nonce
        spam = rate >= cap or abs(skew) > 120000 or size == 0
        lab = "duplicate" if dup else "spam" if spam else "accept"
        if lab != label:
            continue
        return {
            "entry": {"type": rng.choice(ENTRY_TYPES), "submitter": who, "nonce": nonce, "data_hash": dh,
                      "payload_bytes": size, "clock_skew_ms": skew},
            "submitter_recent": {"last_nonce": last_nonce, "recent_data_hashes": recent,
                                 "entries_this_epoch": rate, "epoch_cap": cap},
            "derived": {"nonce_is_next": nonce > last_nonce, "hash_seen_before": dh in recent,
                        "over_cap": rate >= cap, "skew_over_120s": abs(skew) > 120000},
        }
    raise RuntimeError("entry generator failed")


def gen_plaus2(rng, seed, label):
    ctx = g1.gen_plausibility(rng, seed, label)
    vals = [r["value"] for r in ctx["recent"]]
    lo, hi = ctx["physical_range"]
    v = ctx["latest"]["value"]
    med = statistics.median(vals) if vals else None
    steps = [abs(b - a) for a, b in zip(vals, vals[1:])]
    ctx["recent"] = ctx["recent"][-8:]
    ctx["derived"] = {"n_recent": len(vals), "median_recent": None if med is None else round(med, 2),
                      "delta_vs_median": None if med is None else round(v - med, 2),
                      "max_step_recent": round(max(steps), 2) if steps else None,
                      "in_physical_range": lo <= v <= hi}
    return ctx


GEN = {"coverage_usefulness": gen_coverage, "entry_screen": gen_entry, "sensor_plausibility_v2": gen_plaus2}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--seed-json")
    p.add_argument("--out", required=True)
    p.add_argument("--teacher-out", required=True)
    p.add_argument("--n-train", type=int, default=2000)
    p.add_argument("--n-cal", type=int, default=300)
    p.add_argument("--n-test", type=int, default=500)
    a = p.parse_args()
    seed = json.loads(Path(a.seed_json).read_text()) if a.seed_json else {}
    out, tout = Path(a.out), Path(a.teacher_out)
    out.mkdir(parents=True, exist_ok=True); tout.mkdir(parents=True, exist_ok=True)
    for split, n, off in (("train", a.n_train, 0), ("cal", a.n_cal, 1), ("test", a.n_test, 2)):
        rng = random.Random(30261103 * 10 + off)
        rows = []
        for t, gen in GEN.items():
            labels = list(TASKS[t]["candidates"])
            for i in range(n):
                lab = labels[i % len(labels)]
                rows.append({"id": f"{t}-{split}-{i:05d}", "task": t, "split": split, "context": gen(rng, seed, lab),
                             "question": TASKS[t]["question"], "candidates": TASKS[t]["candidates"], "label": lab})
        rng.shuffle(rows)
        with open(out / f"{split}.jsonl", "w") as f, open(tout / f"{split}.jsonl", "w") as ft:
            for r in rows:
                f.write(json.dumps(r, ensure_ascii=False) + "\n")
                k = list(r["candidates"]); pr = [0.02 / (len(k) - 1)] * len(k); pr[k.index(r["label"])] = 0.98
                ft.write(json.dumps({"id": r["id"], "probs": pr}) + "\n")
        print(split, len(rows))


if __name__ == "__main__":
    main()

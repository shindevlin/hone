"""Generate HONE decision-task records with rule-based ground truth.

Usage: python gen_tasks.py --seed-json seed.json --out data/ [--n-train 2000 --n-cal 300 --n-test 500]

seed.json (optional) holds real ids pulled read-only from a HONE node API
(sensor ids/types/owners, node accounts) so contexts carry real names. Values are
synthetic but physically grounded; the label comes from the generating rule.
Each output row: {id, task, split, context, question, candidates{name:desc}, label}.
"""
import argparse, hashlib, json, math, random
from pathlib import Path

HERE = Path(__file__).resolve().parent
TASKS = json.loads((HERE.parent / "tasks" / "hone_tasks.json").read_text())["tasks"]

# kind: (unit, lo, hi, typical_center_range, noise_sd, max_plausible_rate_per_epoch)
KINDS = {
    "barometer": ("hPa", 300.0, 1100.0, (850.0, 1025.0), 0.15, 1.5),
    "temperature": ("C", -60.0, 70.0, (-10.0, 38.0), 0.1, 1.5),
    "humidity": ("%", 0.0, 100.0, (20.0, 90.0), 0.4, 6.0),
    "light": ("lux", 0.0, 130000.0, (5.0, 2000.0), 3.0, 40000.0),
    "accel_magnitude": ("m/s2", 0.0, 160.0, (9.75, 9.85), 0.05, 60.0),
    "magnetometer": ("uT", 0.0, 120.0, (25.0, 65.0), 0.3, 25.0),
    "wifi_ap_count": ("count", 0.0, 400.0, (2.0, 60.0), 1.5, 25.0),
    "subghz_rssi": ("dBm", -130.0, 0.0, (-110.0, -60.0), 2.0, 50.0),
}
CELL_CHARS = "0123456789abcdef"
EPOCH0 = 349000


def cell(rng):
    return "8" + "".join(rng.choice(CELL_CHARS) for _ in range(14))


def rnd(v, kind):
    return int(round(v)) if kind == "wifi_ap_count" else round(v, 2)


def sensor_ident(rng, seed):
    sensors = seed.get("sensors") or []
    if sensors and rng.random() < 0.6:
        s = rng.choice(sensors)
        return s["sensor_id"], s.get("owner", "josh")
    owner = rng.choice(seed.get("accounts") or ["josh", "shindevlin", "natoshisakamoto", "freeport"])
    return f"{owner}-dev{rng.randrange(1, 99)}-{rng.choice(['env', 'imu', 'rf', 'opt'])}", owner


def history(rng, kind, n, center, sd, start_epoch, step):
    drift = rng.gauss(0, sd * 0.3)
    vals, v = [], center
    for i in range(n):
        v = v + drift + rng.gauss(0, sd)
        vals.append({"epoch": start_epoch + i * step, "value": rnd(v, kind)})
    return vals, v


def gen_plausibility(rng, seed, label):
    kind = rng.choice(list(KINDS))
    unit, lo, hi, crange, sd, rate = KINDS[kind]
    sd *= rng.uniform(0.6, 3.0)
    center = rng.uniform(*crange)
    step = rng.choice([1, 2, 10, 20])
    sid, owner = sensor_ident(rng, seed)
    n = rng.randrange(6, 40)
    if label == "unsure" and rng.random() < 0.5:
        n = rng.randrange(0, 2)
    start = EPOCH0 + rng.randrange(0, 5000)
    hist, last = history(rng, kind, n, center, sd, start, step)
    latest_epoch = start + n * step
    if label == "yes":
        v = last + rng.gauss(0, sd * 1.5)
    elif label == "no":
        mode = rng.choice(["limit", "jump", "stuck_impossible"])
        if mode == "limit":
            v = rng.choice([lo - rng.uniform(1, 0.5 * (hi - lo) + 1), hi + rng.uniform(1, 0.5 * (hi - lo) + 1)])
        elif mode == "jump":
            v = last + rng.choice([-1, 1]) * rate * step * rng.uniform(4, 20)
        else:
            v = rng.choice([-9999.0, 65535.0, 0.0 if lo > 0 else 99999.0])
    else:  # unsure
        if n <= 1:
            v = rng.uniform(*crange)
        else:
            v = last + rng.choice([-1, 1]) * max(sd * rng.uniform(8, 25), rate * step * rng.uniform(0.3, 0.8))
            v = min(max(v, lo + 0.01 * (hi - lo)), hi - 0.01 * (hi - lo))
    ctx = {
        "sensor_id": sid, "owner": owner, "kind": kind, "unit": unit,
        "physical_range": [lo, hi], "cell": cell(rng), "epoch_seconds": 30,
        "recent": hist[-rng.choice([4, 8, 16, 40]):] if hist else [],
        "latest": {"epoch": latest_epoch, "value": rnd(v, kind)},
    }
    return ctx


def gen_novelty(rng, seed, label):
    kind = rng.choice(list(KINDS))
    unit, lo, hi, crange, sd, _ = KINDS[kind]
    sd *= rng.uniform(0.6, 2.0)
    center = rng.uniform(*crange)
    sid, owner = sensor_ident(rng, seed)
    c = cell(rng)
    epoch = EPOCH0 + rng.randrange(0, 5000)
    window = [epoch - rng.choice([20, 60, 120]), epoch]
    submitters = seed.get("accounts") or ["josh", "shindevlin", "natoshisakamoto"]
    chain = []
    if label != "new":
        for i in range(rng.randrange(2, 12)):
            chain.append({"epoch": rng.randrange(*window), "value": rnd(center + rng.gauss(0, sd), kind),
                          "submitter": rng.choice(submitters), "cell": c})
        chain.sort(key=lambda r: r["epoch"])
    elif rng.random() < 0.4:
        # comparable readings exist but for another cell or an old window: still new information here
        for i in range(rng.randrange(1, 5)):
            chain.append({"epoch": window[0] - rng.randrange(500, 5000), "value": rnd(center + rng.gauss(0, sd), kind),
                          "submitter": rng.choice(submitters), "cell": cell(rng) if rng.random() < 0.5 else c})
    bh = hashlib.sha256(f"{sid}{epoch}{rng.random()}".encode()).hexdigest()[:16]
    if label == "new":
        v = center + rng.gauss(0, sd)
    elif label == "changed":
        v = center + rng.choice([-1, 1]) * sd * rng.uniform(6, 30)
    else:
        if rng.random() < 0.3:
            dup = rng.choice(chain)
            v, bh = dup["value"], hashlib.sha256(f"{sid}{dup['epoch']}".encode()).hexdigest()[:16]
            dup["batch_hash"] = bh
        else:
            v = center + rng.gauss(0, sd * 0.7)
    ctx = {
        "sensor_id": sid, "kind": kind, "unit": unit, "cell": c, "window_epochs": window,
        "chain_readings": chain,
        "new_reading": {"epoch": epoch, "value": rnd(v, kind), "batch_hash": bh, "submitter": owner},
    }
    return ctx


MODELS = [("smollm2-135m-q8", 160, 60.0), ("qwen2.5-0.5b-q4", 420, 18.0), ("qwen3-1.7b-q4", 1300, 6.0),
          ("qwen3.5-9b-bf16", 18500, 1.0), ("llama3.1-8b-q4", 5200, 2.5), ("open-jev-2b", 4800, 3.0)]
# token-only strings (the jury features gate rejects free text; see docs/JURY.md)
NODES = [("arm64-4xA53-1.2GHz", 0, 25.0), ("arm64-8xA76", 0, 80.0), ("x86_64-20c", 0, 160.0),
         ("x86_64-RTX3090", 24, 2500.0), ("arm64-snapdragon-phone", 0, 60.0)]


def gen_routing(rng, seed, label):
    for _ in range(1000):
        cpu, vram, speed = rng.choice(NODES)
        free = rng.choice([180, 250, 390, 900, 2000, 6000, 14000, 30000])
        local_models = rng.sample([m[0] for m in MODELS], rng.randrange(0, 4))
        model, mram, rel = rng.choice(MODELS)
        tokens = rng.choice([64, 200, 500, 1200, 4000])
        deadline = rng.choice([500, 2000, 10000, 60000, 300000])
        private = rng.random() < 0.15
        queue = rng.randrange(0, 6)
        capable = rng.randrange(0, 12)
        tok_s = speed / mram * 160 if vram == 0 else speed
        est_ms = int(tokens / max(tok_s, 0.1) * 1000) + queue * 400
        fits = model in local_models and (free >= mram or (vram * 1024 >= mram))
        is_local = private or (fits and est_ms <= deadline)
        if not is_local and capable == 0:
            continue
        if (label == "local") == is_local:
            return {
                "job": {"kind": rng.choice(["inference", "embedding", "classify"]), "model": model,
                        "model_ram_mb": mram, "est_tokens": tokens, "deadline_ms": deadline,
                        "private": private},
                "node": {"cpu": cpu, "gpu_vram_gb": vram, "free_ram_mb": free, "local_models": local_models,
                         "queue_len": queue, "est_local_ms": est_ms},
                "network": {"reachable_peers": capable + rng.randrange(0, 10), "capable_peers": capable,
                            "median_rtt_ms": rng.choice([15, 40, 120, 300, 900])},
            }
    raise RuntimeError("routing generator failed")


def gen_peer(rng, seed, label):
    accounts = seed.get("accounts") or ["natoshisakamoto", "shindevlin", "freeport", "bullship"]
    s = {"uptime_pct_24h": round(rng.uniform(92, 100), 1), "clock_skew_ms": int(abs(rng.gauss(0, 120))),
         "msgs_per_min": round(rng.uniform(4, 40), 1), "dup_msgs_1h": rng.randrange(0, 8),
         "invalid_signatures_1h": 0, "conflicting_proposals_24h": 0, "replayed_msgs_1h": 0,
         "missed_epochs_24h": rng.randrange(0, 40), "version": rng.choice(["1.2.2", "1.2.1", "2.0.0"]),
         "stake_hone": round(rng.uniform(0, 500), 2), "is_clock": rng.random() < 0.4}
    if label == "suspicious":
        for k in rng.sample(["skew", "dup", "rate", "missed", "uptime"], rng.randrange(1, 3)):
            if k == "skew": s["clock_skew_ms"] = rng.randrange(2500, 40000)
            if k == "dup": s["dup_msgs_1h"] = rng.randrange(60, 900)
            if k == "rate": s["msgs_per_min"] = round(rng.uniform(300, 3000), 1)
            if k == "missed": s["missed_epochs_24h"] = rng.randrange(900, 2600)
            if k == "uptime": s["uptime_pct_24h"] = round(rng.uniform(20, 70), 1)
    elif label == "misbehaving":
        for k in rng.sample(["sig", "conflict", "replay"], rng.randrange(1, 3)):
            if k == "sig": s["invalid_signatures_1h"] = rng.randrange(3, 400)
            if k == "conflict": s["conflicting_proposals_24h"] = rng.randrange(1, 12)
            if k == "replay": s["replayed_msgs_1h"] = rng.randrange(10, 500)
        if rng.random() < 0.4:
            s["clock_skew_ms"] = rng.randrange(1000, 20000)
    return {"peer": rng.choice(accounts), "window_epochs": 2880, "stats": s,
            "network_medians": {"clock_skew_ms": 80, "msgs_per_min": 18, "dup_msgs_1h": 2, "missed_epochs_24h": 12}}


GEN = {"sensor_plausibility": gen_plausibility, "reading_novelty": gen_novelty,
       "job_routing": gen_routing, "peer_behavior": gen_peer}


def make(task, rng, seed, split, idx):
    labels = list(TASKS[task]["candidates"])
    label = labels[idx % len(labels)]
    ctx = GEN[task](rng, seed, label)
    return {"id": f"{task}-{split}-{idx:05d}", "task": task, "split": split, "context": ctx,
            "question": TASKS[task]["question"], "candidates": TASKS[task]["candidates"], "label": label}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--seed-json")
    p.add_argument("--out", required=True)
    p.add_argument("--n-train", type=int, default=2000)
    p.add_argument("--n-cal", type=int, default=300)
    p.add_argument("--n-test", type=int, default=500)
    p.add_argument("--seed", type=int, default=20261102)
    a = p.parse_args()
    seed = json.loads(Path(a.seed_json).read_text()) if a.seed_json else {}
    out = Path(a.out); out.mkdir(parents=True, exist_ok=True)
    for split, n, off in (("train", a.n_train, 0), ("cal", a.n_cal, 1), ("test", a.n_test, 2)):
        rng = random.Random(a.seed * 10 + off)
        rows = [make(t, rng, seed, split, i) for t in GEN for i in range(n)]
        rng.shuffle(rows)
        with open(out / f"{split}.jsonl", "w") as f:
            for r in rows:
                f.write(json.dumps(r, ensure_ascii=False) + "\n")
        print(split, len(rows))


if __name__ == "__main__":
    main()

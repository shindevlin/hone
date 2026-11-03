"""Fixed parity reference set: Open-Jev's own example (choice + noul + score),
a few generic requests, and 5 rows per HONE task from a fixed-seed generator run.
  python make_refset.py --seed-json seed.json --out refset.jsonl
"""
import argparse, json, random, sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import gen_tasks  # noqa: E402

GENERIC = [
    {"state": "A customer says: I was charged twice for one order. The service is working normally. There is no sign of unauthorized access.",
     "questions": {
         "route": {"type": "choice", "instructions": "Which team should handle this issue?",
                   "criteria": {"billing": "Problems with charges, invoices, refunds or payments.",
                                "security": "Unauthorized access or account compromise.",
                                "technical": "Service unavailable or a software malfunction."}},
         "refund_review": {"type": "noul", "instructions": "Does this message describe a duplicate charge?"},
         "urgency": {"type": "score", "instructions": "Assess the urgency using only the given evidence.",
                     "criteria": ["Routine: no service disruption or active security compromise is reported.",
                                  "Urgent: an ongoing service disruption is reported.",
                                  "Critical: active unauthorized access is reported."]}}},
    {"state": {"node": "nebra", "free_ram_mb": 371, "hone_node_rss_mb": 410, "swap_used_mb": 46},
     "questions": {"ok": {"type": "noul", "instructions": "Is there room to start a 250 MB process without swapping?",
                          "criteria": {"true": "At least 250 MB is available beyond a 100 MB safety margin.",
                                       "false": "Starting it would leave under 100 MB free."}}}},
    {"state": "Sensor moto-gpower-light reported 0 lux at 13:00 local time on a clear day, after 40000 lux one epoch earlier.",
     "questions": {"cause": {"type": "choice", "instructions": "What is the most likely explanation?",
                             "criteria": {"covered": "The phone was put in a pocket or face down.",
                                          "eclipse": "A solar eclipse.", "fault": "The light sensor failed."}},
                   "confidence": {"type": "score", "instructions": "How strongly does this evidence support a sensor fault?",
                                  "criteria": ["No support", "Weak support", "Moderate support", "Strong support"]}}},
    {"state": "Ünïcode check: El Salvador, Antiguo Cuscatlán. Reading at 850 m altitude: 915.4 hPa.",
     "questions": {"plaus": {"type": "noul", "instructions": "Is 915 hPa a normal surface pressure at 850 m altitude?"}}},
]


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--seed-json")
    p.add_argument("--out", required=True)
    p.add_argument("--per-task", type=int, default=5)
    a = p.parse_args()
    seed = json.loads(Path(a.seed_json).read_text()) if a.seed_json else {}
    rows = [{"id": f"generic-{i}", "request": r} for i, r in enumerate(GENERIC)]
    rng = random.Random(777)
    for task in gen_tasks.GEN:
        for i in range(a.per_task):
            r = gen_tasks.make(task, rng, seed, "ref", i)
            if task == "sensor_plausibility" and isinstance(r["context"].get("recent"), list):
                r["context"]["recent"] = r["context"]["recent"][-8:]
            rows.append(r)
    with open(a.out, "w") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(len(rows))


if __name__ == "__main__":
    main()

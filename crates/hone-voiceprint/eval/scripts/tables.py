"""Markdown tables from eval/sim JSON. usage: tables.py EVAL.json [EVAL.json...] -- SIM.json [SIM.json...]"""
import sys, json
a = sys.argv[1:]; i = a.index("--"); evs, sims = a[:i], a[i + 1:]
print("| set | clips | birds | enrol -> test | pipeline | AUC | EER | k=1 P / R | k=5 P / R (held-out tau) |")
print("|---|---|---|---|---|---|---|---|---|")
seen = set()
for f in evs:
    R = json.load(open(f))
    for sp, r in R.items():
        if sp in seen: continue
        seen.add(sp)
        for name in ("raw", "c-raw", "c-rp64"):
            e = r["pipelines"][name]
            o1 = e["op"]["1"]; o5 = e["op"]["5"]
            print(f"| {sp} | {r['n_clips']} | {r['n_ind']} | {r['enrol']} -> {r['test']} | {name} | {e['auc']:.3f} | {e['eer']:.3f} | "
                  f"{o1['precision']:.2f} / {o1['recall']:.2f} | {o5['precision']:.2f} / {o5['recall']:.2f} (tau {o5['tau']}) |")
print()
print("| species (replay) | back events | back precision | false back | back recall | call precision | birds -> with id | ids per bird | mean likelihood right / wrong |")
print("|---|---|---|---|---|---|---|---|---|")
for f in sims:
    for sp, r in json.load(open(f)).items():
        print(f"| {sp} | {r['back_events']} | {r['back_precision']} | {r['false_back_rate']} | {r['back_recall']} | {r['call_precision']} | "
              f"{r['individuals']} -> {r['with_candidate']} | {r['ids_per_individual']} | {r['mean_lik_correct']} / {r['mean_lik_wrong']} |")

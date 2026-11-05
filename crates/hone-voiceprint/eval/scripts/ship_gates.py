"""Shipped gates.json: generic dev-tuned clustering params, per-species logistic from eval5/eval6,
naming_allowed only where the held-out gate passed (see RESULTS.md)."""
import json
e5 = json.load(open("/home/beastly/voiceprint-eval/out/eval5.json"))
e6 = json.load(open("/home/beastly/voiceprint-eval/out/eval6.json"))
sim = json.load(open("/home/beastly/voiceprint-eval/out/sim-final.json"))
sim.update(json.load(open("/home/beastly/voiceprint-eval/out/sim-kiwi.json")))
rows = [("Athene noctua", e5, "littleowl", "Athene_noctua", True),
        ("Anthus trivialis", e5, "pipit", "Anthus_trivialis", False),
        ("Phylloscopus collybita", e5, "chiffchaff_year", "Phylloscopus_collybita@year", False),
        ("Parus major", e5, "greattit_days", "Parus_major", False),
        ("Apteryx haastii", e6, "kiwi_seasons", "Apteryx_haastii", False),
        ("Pitangus sulphuratus", e5, "kiskadee_halves", None, False)]
g = {}
for sp, ev, key, simkey, ok in rows:
    e = ev[key]["pipelines"]["c-rp64"]
    s = sim.get(simkey) if simkey else None
    src = f"{key}: AUC {e['auc']}"
    if s: src += f"; replay gap P {s['gap_back_precision']} R {s['gap_back_recall']} n {s['gap_back_events']}"
    if sp == "Pitangus sulphuratus": src += "; same-recording halves only (upper bound), no cross-night data"
    g[sp] = dict(naming_allowed=ok, join_tau=0.62, back_tau=0.6, k_min=5, merge_tau=0.75, split_tau=0.2,
                 logit_a=round(e["logit_a"], 3), logit_b=round(e["logit_b"], 3), source=src)
out = "/home/beastly/vp-src/crates/hone-voiceprint/eval/gates.json"
json.dump({"species": g}, open(out, "w"), indent=1)
print(json.dumps(g, indent=1))

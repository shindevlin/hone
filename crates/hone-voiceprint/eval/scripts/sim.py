"""End-to-end test of the real crate (examples/vp_sim) on labelled streams.

usage: sim.py OUT.json GATES.json SPECIES_KEY=npz:mode[:extra] ...
  mode "sessions:A,B"  - clips of session A spread over nights 0-2, session B over nights 3-5
  mode "dates"         - npz session field holds the real recording date (YYYYMMDD) = night
A "home" is a random group of 2-5 individuals heard in one place (one population). 40 homes.
Labels never enter the crate; they are only used to score what it says.
Scores (pooled over homes):
  back precision = "X is back" events whose night-calls are mostly X's owner / all back events
  false back     = back events on a night when the owner bird was not recorded at all
  back recall    = (owner, night) pairs, owner present with >= k_min calls on a night after the
                   candidate formed, that got a correct back event
  call precision = per-call attributions (Match.bird_id) that are the owner's calls
  fragmentation  = candidate ids per individual that got any
"""
import sys, json, struct, subprocess, numpy as np, collections, random, datetime
OUT, GATES = sys.argv[1], sys.argv[2]
SIM = "/home/beastly/vp-src/crates/hone-voiceprint/target/release/examples/vp_sim"
CELL = "8966c2d6b6bffff"
gates = json.load(open(GATES))["species"]
res = {}
for arg in sys.argv[3:]:
    key, rest = arg.split("=")
    npz, mode = rest.split(":", 1)
    sp = key.split("@")[0].replace("_", " ")
    kmin = gates.get(sp, {}).get("k_min", 5)
    z = np.load(npz)
    E, L, S = z["emb"], z["label"].astype(str), z["session"].astype(str)
    if mode.startswith("sessions:"):
        A, B = mode.split(":")[1].split(",")
        keep = np.isin(S, [A, B]); E, L, S = E[keep], L[keep], S[keep]
    rng = random.Random(11)
    inds = sorted(set(L))
    tot = collections.Counter(); frag = []
    for home in range(40):
        group = rng.sample(inds, min(len(inds), rng.randint(2, 5)))
        recs = []
        for g in group:
            ix = np.where(L == g)[0]
            if mode == "dates":
                for i in ix:
                    d = datetime.date(int(S[i][:4]), int(S[i][4:6]), int(S[i][6:8]))
                    recs.append((d.toordinal(), i))
            else:
                a = [i for i in ix if S[i] == A]; b = [i for i in ix if S[i] == B]
                rng.shuffle(a); rng.shuffle(b)
                recs += [(n * 3 // max(1, len(a)), i) for n, i in enumerate(a)]
                recs += [(3 + n * 3 // max(1, len(b)), i) for n, i in enumerate(b)]
        # nights -> timestamps: dawn, calls a minute apart, birds interleaved
        recs.sort(key=lambda r: (r[0], rng.random()))
        base = datetime.datetime(2026, 1, 1, 18).timestamp() * 1000
        first = recs[0][0]
        buf = bytearray(); order = []
        per_night = collections.Counter()
        for nd, i in recs:
            k = per_night[nd]; per_night[nd] += 1
            t = int(base + (nd - first) * 86400000 + k * 30000)
            buf += struct.pack("<IQf", int(i), t, 0.9) + E[i].astype("<f4").tobytes()
            order.append((nd - first, i))
        p = subprocess.run([SIM, sp, CELL, GATES], input=bytes(buf), capture_output=True, check=True)
        lines = [json.loads(l) for l in p.stdout.decode().splitlines()]
        rows = [l for l in lines if "label" in l]
        events = [l for l in lines if "events" in l][0]["events"]
        # owner of each bird id = majority label over its attributed calls
        # owner = the bird the user would have named: majority of the first 10 calls attributed
        # to the id once it exists (so a later identity swap counts as wrong, not as a new owner)
        att = collections.defaultdict(collections.Counter)
        for r in rows:
            if r["bird"] and sum(att[r["bird"]].values()) < 10:
                att[r["bird"]][L[r["label"]]] += 1
        owner = {b: c.most_common(1)[0][0] for b, c in att.items()}
        for r in rows:
            if r["bird"]:
                tot["calls_attr"] += 1; tot["calls_ok"] += L[r["label"]] == owner[r["bird"]]
        # night of each row, calls per (night, label)
        night_of_row = {}
        present = collections.defaultdict(collections.Counter)
        for (nd, i), r in zip(order, rows):
            night_of_row[r["i"]] = nd; present[nd][L[i]] += 1
        cand_night = {}
        for e in events:
            ev = e["ev"]
            if ev["kind"] == "candidate" and e["i"] is not None:
                cand_night.setdefault(ev["bird_id"], night_of_row[e["i"]])
        good_pairs = set()
        for e in events:
            ev = e["ev"]
            if ev["kind"] != "back" or e["i"] is None:
                continue
            b = ev["bird_id"]; nd = night_of_row[e["i"]]
            if mode != "dates" and nd < 3:
                continue
            tot["back"] += 1
            o = owner.get(b)
            # calls attributed to b that night
            c = collections.Counter(L[r["label"]] for r in rows if r["bird"] == b and night_of_row[r["i"]] == nd)
            ok = o is not None and c and c.most_common(1)[0][0] == o
            tot["back_ok"] += bool(ok)
            cn = cand_night.get(b)
            if cn is not None and ((mode != "dates" and cn < 3) or (mode == "dates" and nd - cn >= 2)):
                tot["gap_back"] += 1; tot["gap_ok"] += bool(ok)
            if o is None or present[nd][o] == 0:
                tot["false_back"] += 1
            if ok:
                good_pairs.add((o, nd))
            tot["lik_sum_ok" if ok else "lik_sum_bad"] += ev["likelihood"]
        for b, o in owner.items():
            if b not in cand_night:
                continue
            for nd in present:
                if nd > cand_night[b] and present[nd][o] >= kmin and (mode == "dates" or nd >= 3):
                    tot["back_opps"] += 1; tot["back_hit"] += (o, nd) in good_pairs
                    if (mode != "dates" and cand_night[b] < 3) or (mode == "dates" and nd - cand_night[b] >= 2):
                        tot["gap_opps"] += 1; tot["gap_hit"] += (o, nd) in good_pairs
        by_ind = collections.Counter(owner.values())
        frag += list(by_ind.values())
        tot["individuals"] += len(group); tot["with_candidate"] += len(by_ind)
    r = dict(
        homes=40,
        back_events=tot["back"],
        back_precision=round(tot["back_ok"] / tot["back"], 3) if tot["back"] else None,
        false_back_rate=round(tot["false_back"] / tot["back"], 3) if tot["back"] else None,
        back_recall=round(tot["back_hit"] / tot["back_opps"], 3) if tot["back_opps"] else None,
        gap_back_events=tot["gap_back"],
        gap_back_precision=round(tot["gap_ok"] / tot["gap_back"], 3) if tot["gap_back"] else None,
        gap_back_recall=round(tot["gap_hit"] / tot["gap_opps"], 3) if tot["gap_opps"] else None,
        call_precision=round(tot["calls_ok"] / tot["calls_attr"], 3) if tot["calls_attr"] else None,
        individuals=tot["individuals"], with_candidate=tot["with_candidate"],
        ids_per_individual=round(float(np.mean(frag)), 2) if frag else None,
        mean_lik_correct=round(tot["lik_sum_ok"] / max(1, tot["back_ok"]), 3),
        mean_lik_wrong=round(tot["lik_sum_bad"] / max(1, tot["back"] - tot["back_ok"]), 3),
    )
    res[key] = r
    print(key, r, flush=True)
json.dump(res, open(OUT, "w"), indent=1)

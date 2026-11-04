"""Derive enrol/test sessions.
  kiskadee: frames passing the phone gates (conf >= 0.6, human <= 0.05); per recording the first
            40% of frames -> "A", the last 40% -> "B" (middle dropped so overlapping 5 s frames
            never straddle). Needs >= 2 frames each side. Same bird, same session: an UPPER bound.
  greattit: per bird, "early" = first recording date, "late" = dates >= first + 2 days;
            "sameA"/"sameB" = first date's files split in halves (to measure same-day optimism).
  kiwi:     per bird, earliest season vs later season (> 60 days later).
Each output npz keeps emb/label/session/clip like embed.py, with dates kept in "date" when known."""
import numpy as np, datetime
H = "/home/beastly/voiceprint-eval/out/"

def save(name, idx, z, sess, extra=None):
    d = dict(emb=z["emb"][idx], label=z["label"][idx], session=np.array(sess), clip=z["clip"][idx])
    if extra is not None: d["date"] = np.array(extra)
    np.savez_compressed(H + name, **d); print(name, len(idx), dict(zip(*np.unique(sess, return_counts=True))), "birds", len(set(z["label"][idx])))

try:
    z = np.load(H + "kiskadee.npz")
    ok = (z["conf"] >= 0.6) & (z["human"] <= 0.05)
    idx, sess = [], []
    for c in np.unique(z["clip"]):
        f = np.where((z["clip"] == c) & ok)[0]
        n = len(f); a = f[: int(n * 0.4)]; b = f[n - int(n * 0.4):]
        if len(a) >= 2 and len(b) >= 2:
            idx += list(a) + list(b); sess += ["A"] * len(a) + ["B"] * len(b)
    # each frame is its own clip for the evaluator
    zz = dict(z); zz["clip"] = np.arange(len(z["clip"]))
    save("kiskadee_halves.npz", np.array(idx), zz, sess)
except FileNotFoundError:
    print("no kiskadee yet")

try:
    z = np.load(H + "greattit.npz")
    dates = np.array([datetime.date(int(s[:4]), int(s[4:6]), int(s[6:8])).toordinal() for s in z["session"].astype(str)])
    idx, sess, idx2, sess2 = [], [], [], []
    for b in np.unique(z["label"]):
        f = np.where(z["label"] == b)[0]
        # clip level: one frame per clip is enough (songs are short)
        d0 = dates[f].min()
        early = [i for i in f if dates[i] == d0]; late = [i for i in f if dates[i] >= d0 + 2]
        if len(early) >= 3 and len(late) >= 3:
            idx += early + late; sess += ["early"] * len(early) + ["late"] * len(late)
            h = len(early) // 2
            idx2 += early; sess2 += ["sameA"] * h + ["sameB"] * (len(early) - h)
    save("greattit_days.npz", np.array(idx), z, sess)
    save("greattit_sameday.npz", np.array(idx2), z, sess2)
except FileNotFoundError:
    print("no greattit yet")

try:
    z = np.load(H + "kiwi.npz")
    dates = np.array([datetime.date(int(s[:4]), int(s[4:6]), int(s[6:8])).toordinal() for s in z["session"].astype(str)])
    idx, sess = [], []
    for b in np.unique(z["label"]):
        f = np.where(z["label"] == b)[0]; d0 = dates[f].min()
        early = [i for i in f if dates[i] <= d0 + 30]; late = [i for i in f if dates[i] > d0 + 60]
        if len(early) >= 2 and len(late) >= 2:
            idx += early + late; sess += ["early"] * len(early) + ["late"] * len(late)
    save("kiwi_seasons.npz", np.array(idx), z, sess)
except FileNotFoundError:
    print("no kiwi yet")

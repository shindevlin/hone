"""Embed lists for kiskadee (xeno-canto via GBIF), great tit and kiwi (zenodo 20145425).
kiskadee: label = XC id, session = "rec" (halves are split later by frame order).
great tit: label = ringed father, session = recording date. kiwi: label = individual dir, session = date."""
import os, glob, re, pandas as pd
H = "/home/beastly/voiceprint-eval"
os.makedirs(H + "/lists", exist_ok=True)
rows = [l.rstrip("\n").split("\t") for l in open(H + "/data/xc/Pitangus_sulphuratus/manifest.tsv")]
open(H + "/lists/kiskadee.txt", "w").write("".join(f"{r[0]}\trec\t{r[1]}\n" for r in rows if os.path.exists(r[1])))
gt = pd.read_csv(H + "/data/gallego/greattit_index.csv")
fa = dict(zip(gt.filename.str.lower(), gt.father))
out = []
for p in sorted(glob.glob(H + "/data/gallego/audio/greatTit_song-files/**/*.*", recursive=True)):
    b = os.path.basename(p).rsplit(".", 1)[0]
    if b.lower() in fa:
        out.append(f"{fa[b.lower()]}\t{b.split('_')[1]}\t{p}")
open(H + "/lists/greattit.txt", "w").write("\n".join(out) + "\n")
out = []
for p in sorted(glob.glob(H + "/data/gallego/audio/KiwiTrimmed/*/*.wav")):
    ind = p.split('/')[-2]
    if ind.endswith("99"):
        continue  # F99 / M99: not individually identified
    b = os.path.basename(p)
    m = re.search(r"_(20\d{6})_", b) or re.match(r"\d+_(20\d\d)_(\d\d)_(\d\d)_", b)
    if m:
        d = m.group(1) if m.lastindex == 1 else m.group(1) + m.group(2) + m.group(3)
        out.append(f"{ind}\t{d}\t{p}")
open(H + "/lists/kiwi.txt", "w").write("\n".join(out) + "\n")
for n in ["kiskadee", "greattit", "kiwi"]:
    print(n, sum(1 for _ in open(f"{H}/lists/{n}.txt")))

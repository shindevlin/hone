"""Pull selected audio out of zenodo 20145425 Datasets.zip (CC BY 4.0 audio) with HTTP ranges.
Great tit (Wytham Woods, ringed fathers): 48 birds with >= 5 recording days, <= 12 files per day.
Kiwi (great spotted kiwi, KiwiTrimmed): everything except _qc."""
import os, random, pandas as pd
from remotezip import RemoteZip
root = "/home/beastly/voiceprint-eval/data/gallego/audio"
os.makedirs(root, exist_ok=True)
idx = [l.split("\t")[0] for l in open("/home/beastly/voiceprint-eval/data/gallego/datasets_index.tsv")]
gt = pd.read_csv("/home/beastly/voiceprint-eval/data/gallego/greattit_index.csv")
days = gt.groupby("father").date.nunique()
birds = sorted(days[days >= 5].index)
random.Random(3).shuffle(birds); birds = sorted(birds[:48])
pick = []
for b in birds:
    for d, g in gt[gt.father == b].groupby("date"):
        fs = sorted(g.filename); random.Random(hash((b, d)) & 0xffff).shuffle(fs)
        pick += fs[:12]
pick = set(f.lower() for f in pick)
want = [n for n in idx if (n.startswith("Originaldatasets/greatTit_song-files/") and n.rsplit("/", 1)[-1].rsplit(".", 1)[0].lower() in pick)
        or (n.startswith("Originaldatasets/KiwiTrimmed/") and "/_qc" not in n and n.endswith(".wav"))]
import sys
if os.environ.get("KIWI_ONLY"):
    want = [n for n in want if "KiwiTrimmed" in n]
if len(sys.argv) > 2:
    k, n = int(sys.argv[1]), int(sys.argv[2]); want = want[k::n]
print("want", len(want), flush=True)
done = 0
with RemoteZip("https://zenodo.org/records/20145425/files/Datasets.zip?download=1") as z:
    for n in want:
        out = os.path.join(root, n.replace("Originaldatasets/", ""))
        if os.path.exists(out):
            continue
        os.makedirs(os.path.dirname(out), exist_ok=True)
        import time
        for attempt in range(12):
            try:
                open(out + ".part", "wb").write(z.read(n)); os.rename(out + ".part", out); break
            except Exception as e:
                print("retry", attempt, n.rsplit("/", 1)[-1], str(e)[:40], flush=True)
                time.sleep(20 * (attempt + 1))
        time.sleep(0.3)
        done += 1
        if done % 100 == 0:
            print(done, flush=True)
print("ALLDONE", done)

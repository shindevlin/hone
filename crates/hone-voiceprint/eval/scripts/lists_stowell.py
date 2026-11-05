"""Build embed lists from the Stowell et al. CSVs (CC BY 4.0, zenodo 1413495).
session = "<split>-<trn|tst>", label = individual id. Writes lists/<species>.txt"""
import csv, os, glob
root = os.path.expanduser("~/voiceprint-eval/data/stowell")
os.makedirs(os.path.expanduser("~/voiceprint-eval/lists"), exist_ok=True)
wav = {os.path.basename(p): p for p in glob.glob(root + "/wav/**/*.wav", recursive=True)}
for sp in ["littleowl", "pipit", "chiffchaff"]:
    out, seen, miss = [], set(), 0
    for split in ["acrossyear", "withinyear"]:
        for part in ["trn", "tst"]:
            f = f"{root}/csv/csv/{sp}-{split}-fg-{part}.csv"
            if not os.path.exists(f):
                continue
            r = list(csv.reader(open(f)))
            hdr = r[0]
            for row in r[1:]:
                lab = [hdr[i] for i in range(1, len(row)) if row[i].strip() == "1"]
                if not lab or row[0] in seen:
                    continue
                if row[0] not in wav:
                    miss += 1; continue
                seen.add(row[0])
                out.append(f"{lab[0]}\t{split}-{part}\t{wav[row[0]]}")
    open(os.path.expanduser(f"~/voiceprint-eval/lists/{sp}.txt"), "w").write("\n".join(out) + "\n")
    print(sp, len(out), "missing", miss)

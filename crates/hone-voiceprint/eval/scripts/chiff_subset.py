import random
H = "/home/beastly/voiceprint-eval/lists/"
rows = [l.rstrip("\n").split("\t") for l in open(H + "chiffchaff.txt") if l.strip()]
day1 = [r for r in rows if "_day1_" in r[2]]
rest = [r for r in rows if "_day1_" not in r[2]]
random.Random(5).shuffle(day1)
# keep up to 70 day1 clips per bird
per = {}
keep = []
for r in day1:
    if per.get(r[0], 0) < 70:
        per[r[0]] = per.get(r[0], 0) + 1; keep.append(r)
out = rest + keep
open(H + "chiffchaff_sub.txt", "w").write("".join("\t".join(r) + "\n" for r in out))
import collections
print(len(out), collections.Counter(r[1] for r in out), collections.Counter(r[2].split("/")[-1].split("_")[1] for r in out))

"""Fetch Great Kiskadee recordings from xeno-canto via GBIF (dataset b1047888-...).
Evaluation only: files are not redistributed. Licence, recordist, place and date of every file
are written to data/xc/manifest.tsv.   usage: fetch_xc.py "Pitangus sulphuratus" N"""
import json, os, subprocess, sys, urllib.request, urllib.parse
sp, N = sys.argv[1], int(sys.argv[2])
root = os.path.expanduser("~/voiceprint-eval/data/xc/" + sp.replace(" ", "_"))
os.makedirs(root, exist_ok=True)
man = open(root + "/manifest.tsv", "a")
have = {l.split("\t")[0] for l in open(root + "/manifest.tsv")} if os.path.getsize(root + "/manifest.tsv") else set()
got, off, seen_rec = len(have), 0, set()
while got < N and off < 2000:
    q = urllib.parse.urlencode({"datasetKey": "b1047888-ae52-4179-9dd5-5448ea342a24", "scientificName": sp,
                                "mediaType": "Sound", "limit": 100, "offset": off})
    d = json.load(urllib.request.urlopen("https://api.gbif.org/v1/occurrence/search?" + q))
    if not d["results"]:
        break
    for r in d["results"]:
        xc = r.get("occurrenceID", "").rsplit("/", 1)[-1]
        snd = [m for m in r.get("media", []) if m.get("type") == "Sound" or str(m.get("identifier", "")).endswith((".mp3", ".wav"))]
        if not xc or xc in have or not snd:
            continue
        who = r.get("recordedBy", "")
        # one recording per recordist+place+day, so different files are (almost surely) different birds
        k = (who, round(r.get("decimalLatitude") or 0, 1), round(r.get("decimalLongitude") or 0, 1))
        if k in seen_rec:
            continue
        url = snd[0]["identifier"]; ext = url.rsplit(".", 1)[-1].lower()
        out = f"{root}/{xc}.{ext}"
        rc = subprocess.run(["curl", "-fsSL", "--max-filesize", "20000000", "-o", out, url]).returncode
        if rc != 0:
            if os.path.exists(out): os.remove(out)
            continue
        seen_rec.add(k); have.add(xc); got += 1
        man.write("\t".join([xc, out, snd[0].get("license", r.get("license", "")), who, str(r.get("country")),
                             str(r.get("decimalLatitude")), str(r.get("decimalLongitude")), str(r.get("eventDate"))]) + "\n")
        man.flush()
        print(got, xc, flush=True)
        if got >= N:
            break
    off += 100
print("done", got)

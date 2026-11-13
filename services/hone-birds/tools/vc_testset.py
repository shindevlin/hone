"""Voice-check test set: real bird recordings with real speech mixed in (8 languages, 3 levels) + clean birds.
Builds in a scratch folder, then scores every clip on the real path (Opus clip -> decode -> Silero) and compares
detector settings on the same clips: false alarms on clean birds vs speech found per level and language.
Audio sources are used for this test only. Usage (indexer folder): python -I vc_testset.py <scratch_dir>"""
import io, json, os, random, sys, time, urllib.error, urllib.parse, urllib.request
import numpy as np
import av

sys.path.insert(0, "/opt/hone-birds")
import hone_voicecheck as V

OUT = sys.argv[1]
os.makedirs(OUT, exist_ok=True)
UA = {"User-Agent": "HoneVoiceCheckTest/1.1 (https://honemesh.net; internal speech-filter test)"}
SR = 16000
random.seed(7)


def fetch(url, cache, pause=2.0):
    p = os.path.join(OUT, cache)
    if os.path.exists(p) and os.path.getsize(p) > 0:
        return open(p, "rb").read()
    for attempt in range(5):
        try:
            data = urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=60).read()
            if data:
                open(p, "wb").write(data)
            time.sleep(pause)
            return data
        except urllib.error.HTTPError as e:
            if e.code != 429:
                raise
            time.sleep(10 * (attempt + 1))
    raise RuntimeError("rate limited: " + url[:60])


def pcm_of(data, max_s=20):
    out = []
    with av.open(io.BytesIO(data)) as c:
        rs = av.AudioResampler(format="flt", layout="mono", rate=SR)
        try:
            for fr in c.decode(c.streams.audio[0]):
                for f in rs.resample(fr):
                    out.append(f.to_ndarray().reshape(-1))
                if sum(map(len, out)) > max_s * SR:
                    break
        except av.error.FFmpegError:
            pass
    return np.concatenate(out)[: max_s * SR].astype(np.float32) if out else np.zeros(0, np.float32)


def opus(pcm):
    b = io.BytesIO()
    with av.open(b, "w", format="ogg") as c:
        s = c.add_stream("libopus", rate=48000)
        s.layout = "mono"
        fr = av.AudioFrame.from_ndarray((pcm.clip(-1, 1) * 32767).astype(np.int16)[None, :], format="s16", layout="mono")
        fr.sample_rate = SR
        rs = av.AudioResampler(format="s16", layout="mono", rate=48000)
        for f in list(rs.resample(fr)) + list(rs.resample(None)):
            for pkt in s.encode(f):
                c.mux(pkt)
        for pkt in s.encode(None):
            c.mux(pkt)
    return b.getvalue()


def rms(x):
    return float(np.sqrt(np.mean(x.astype(np.float64) ** 2))) if len(x) else 0.0


SPECIES = ["Anas discors", "Dendrocygna autumnalis", "Amazona albifrons", "Amazona auropalliata", "Turdus grayi",
           "Quiscalus mexicanus", "Pitangus sulphuratus", "Columbina inca", "Leucophaeus atricilla", "Megascops guatemalae"]
birds = []
for sp in SPECIES:
    try:
        recs = json.loads(urllib.request.urlopen("http://127.0.0.1:9460/api/birds/call/" + urllib.parse.quote(sp), timeout=60).read())
    except Exception as e:
        print("no calls for", sp, e); continue
    n = 0
    for r in recs.get("recordings", []):
        url = r.get("file_url")
        if not url:
            continue
        try:
            x = pcm_of(fetch(url, f"bird-{sp.replace(' ', '_')}-{n}.audio", 1.0))
        except Exception as e:
            print("skip", sp, str(e)[:60]); continue
        if len(x) >= 6 * SR and rms(x) > 1e-4:
            birds.append((sp, (x / max(1e-6, np.abs(x).max()) * 0.5)[: 15 * SR])); n += 1
        if n == 2:
            break
print("bird clips:", len(birds), flush=True)

LANGS = ["eng", "spa", "fra", "deu", "cmn", "ara", "hin", "por"]
API = "https://commons.wikimedia.org/w/api.php?"
speech = {}
for lg in LANGS:
    d = json.loads(fetch(API + urllib.parse.urlencode(
        {"action": "query", "generator": "categorymembers", "gcmtitle": f"Category:Lingua Libre pronunciation-{lg}",
         "gcmtype": "file", "gcmlimit": 40, "prop": "imageinfo", "iiprop": "url", "format": "json"}), f"list-{lg}.json"))
    urls = [p["imageinfo"][0]["url"] for p in d.get("query", {}).get("pages", {}).values() if p.get("imageinfo")]
    random.shuffle(urls)
    words = []
    for i, u in enumerate(urls[:20]):
        try:
            w = pcm_of(fetch(u, f"speech2-{lg}-{i}.wav"), 6)
        except Exception as e:
            continue
        loud = np.nonzero(np.abs(w) > 0.05 * max(1e-6, np.abs(w).max()))[0]
        w = w[loud[0]:loud[-1] + 1] if len(loud) else w[:0]
        if 0.25 * SR < len(w) <= 4 * SR and rms(w) > 1e-3:
            words.append(w / max(1e-6, np.abs(w).max()) * 0.5)
        if len(words) == 4:
            break
    speech[lg] = words
    print("speech", lg, len(words), flush=True)

LEVELS = {"loud (-6 dB)": 0.5, "quiet (-18 dB)": 0.126, "distant (-30 dB)": 0.0316}
clips = []   # (kind, species, lang, probs)
for k, (sp, b) in enumerate(birds):
    clips.append(("clean", sp, "-", V.probabilities(pcm_of(opus(b)))))
    lg = LANGS[k % len(LANGS)]
    words = speech.get(lg) or []
    if not words:
        continue
    gap = np.zeros(int(0.15 * SR), np.float32)
    phrase = np.concatenate([np.concatenate([w, gap]) for w in random.sample(words, min(3, len(words)))])
    at = random.randrange(0, max(1, len(b) - len(phrase)))
    for name, rel in LEVELS.items():
        mix = b.copy()
        seg = phrase[: len(mix) - at] * (rel * rms(b) / max(1e-6, rms(phrase[np.abs(phrase) > 0.01])))
        mix[at:at + len(seg)] += seg
        assert rms(mix - b) > 0, "speech not mixed in"
        clips.append((name, sp, lg, V.probabilities(pcm_of(opus(mix / max(1.0, np.abs(mix).max()))))))
json.dump([(a, b_, c, [round(p, 3) for p in d]) for a, b_, c, d in clips], open(os.path.join(OUT, "probs.json"), "w"))


def speech_found(ps, bar, run):
    n = 0
    for p in ps:
        n = n + 1 if p >= bar else 0
        if n >= run:
            return True
    return False


print("\nsetting                      | false alarms (clean) | " + " | ".join(LEVELS))
for bar, run in [(0.35, 1), (0.5, 1), (0.5, 4), (0.5, 8), (0.4, 8), (0.35, 8), (0.6, 8), (0.5, 12)]:
    fa = [c for c in clips if c[0] == "clean" and speech_found(c[3], bar, run)]
    row = [f"{sum(speech_found(c[3], bar, run) for c in clips if c[0] == lv)}/{sum(1 for c in clips if c[0] == lv)}" for lv in LEVELS]
    print(f"bar {bar:<4} run {run:>2} ({run * 32:>3} ms) | {len(fa)}/{sum(1 for c in clips if c[0] == 'clean')}"
          f" {[c[1].split()[0] for c in fa]} | " + " | ".join(row), flush=True)
best = (0.5, 8)
print("\nper language at bar 0.5, run 8:")
for lg in LANGS:
    rows = [c for c in clips if c[2] == lg]
    if rows:
        print(f"  {lg}: {sum(speech_found(c[3], *best) for c in rows)}/{len(rows)}")

"""Evaluate the Hone voice gate: speech leak (must be < 1 %) and cost to birds."""
import glob, json, os, re, sys, time
import numpy as np, soundfile as sf
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import voice_gate as vg
from ai_edge_litert.interpreter import Interpreter

PERCH = os.environ.get("PERCH_DIR", os.path.expanduser("~/perch"))
SR, FRAME, BLOCK = 32000, 160000, 3200
HUMAN = re.compile(r"(?i)speech|speaking|voice|conversation|whisper|singing|shout|scream|laugh|giggle|"
                   r"chuckle|chatter|crowd|crying|babbl|cough|sneeze|^homo sapiens$|human")
c = json.load(open(f"{PERCH}/assets/class_names.json")); names = {v - 1: k for k, v in c.items()}
human_idx = np.array([i for i, n in names.items() if HUMAN.search(n)])
it = Interpreter(f"{PERCH}/perch_v2_8bit.tflite", num_threads=4); it.allocate_tensors()
inp = it.get_input_details()[0]; lab = [o for o in it.get_output_details() if o["shape"][-1] == 14795][0]


def perch(x):
    it.set_tensor(inp["index"], x.astype(np.float32)[None]); it.invoke()
    return it.get_tensor(lab["index"])[0]


def rms_db(x): return 20 * np.log10(np.sqrt(np.mean(x ** 2)) + 1e-12)
def at_level(x, db): return x * (10 ** ((db - rms_db(x)) / 20))
def up2(a): t = np.arange(len(a) * 2) / 2.0; return np.interp(t, np.arange(len(a)), a).astype(np.float32)


def load(path):
    a, sr = sf.read(path, dtype="float32", always_2d=True)
    a = a.mean(1)
    if sr != SR:
        t = np.arange(int(len(a) * SR / sr)) * (sr / SR)
        a = np.interp(t, np.arange(len(a)), a).astype(np.float32)
    return a


def frames_of(a, n=None):
    fr = [a[i:i + FRAME] for i in range(0, len(a) - FRAME + 1, FRAME)]
    fr = [f for f in fr if rms_db(f) > -60]
    return fr[:n] if n else fr


# ── data ──
speech_files = sorted(glob.glob(sys.argv[1] + "/**/*.flac", recursive=True))
N = int(sys.argv[3]) if len(sys.argv) > 3 else 300
sp = []
for f in speech_files:
    sp.append(up2(sf.read(f, dtype="float32")[0]))
    if sum(map(len, sp)) >= N * FRAME: break
speech = frames_of(np.concatenate(sp), N)
skip = re.compile(r"(?i)kasse|sheep|interview|speech")
bird_files = [p for p in sorted(glob.glob(sys.argv[2] + "/*")) if not p.endswith(".json") and not skip.search(p)]
birds = []
for p in bird_files:
    try:
        for f in frames_of(load(p))[:6]:
            birds.append((p.split("/")[-1], f))
    except Exception as e:
        print("skip", p, e)
print(f"speech frames {len(speech)}, bird frames {len(birds)} from {len(bird_files)} files")


def speech_blocks(clean):
    # ground truth: block carries speech if its RMS is within 20 dB of the frame's speech RMS
    ref = rms_db(clean)
    return np.array([rms_db(clean[b * BLOCK:(b + 1) * BLOCK]) > ref - 20 for b in range(FRAME // BLOCK)])


def run_speech(label, mk):
    total = leaked = frames_kept = 0
    t0 = time.time()
    for k, s in enumerate(speech):
        clean, mix = mk(s, k)
        truth = speech_blocks(clean)
        mask, drop = vg.speech_mask(mix)
        if not drop:
            out = vg.apply(mix, mask)
            hp = float(perch(out)[human_idx].max())
            if hp > 0.05:      # second gate: Perch human classes
                drop = True
        total += truth.sum()
        if not drop:
            frames_kept += 1
            leaked += (truth & ~mask).sum()
    rate = 100.0 * leaked / max(total, 1)
    print(f"{label:48} speech blocks leaked {leaked}/{total} = {rate:.2f} %  (frames not dropped {frames_kept}/{len(speech)})  {time.time()-t0:.0f}s")
    return {"leaked_blocks": int(leaked), "speech_blocks": int(total), "leak_pct": round(rate, 3), "frames_kept": frames_kept}


res = {"params": {k: getattr(vg, k) for k in ["VOICED_R", "FLOOR_DB", "MIN_VOICED", "MARGIN_BLOCKS", "DROP_FRACTION", "LAG_MIN", "LAG_MAX"]}}
bird_pool = [b for _, b in birds]
res["speech_normal"] = run_speech("speech normal (-26 dBFS)", lambda s, k: (at_level(s, -26),) * 2)
res["speech_quiet"] = run_speech("speech quiet (-40 dBFS)", lambda s, k: (at_level(s, -40),) * 2)
def under(s, k):
    cl = at_level(s, -32); return cl, cl + at_level(bird_pool[k % len(bird_pool)], -26)
res["speech_under_birds"] = run_speech("speech under birdsong (-32 vs -26)", under)

# ── cost to birds ──
muted = blocks = dropped = kept_top = 0
per_file = {}
for name, b in birds:
    x = at_level(b, -26)
    mask, drop = vg.speech_mask(x)
    p0 = perch(x); top0 = int(p0.argmax())
    out = vg.apply(x, mask)
    if not drop:
        p1 = perch(out); drop = float(p1[human_idx].max()) > 0.05
        same = (not drop) and int(p1.argmax()) == top0
    else:
        same = False
    muted += mask.sum(); blocks += len(mask); dropped += drop; kept_top += same
    d = per_file.setdefault(name, [0, 0, 0]); d[0] += mask.sum(); d[1] += len(mask); d[2] += drop
res["birds"] = {"frames": len(birds), "blocks_muted_pct": round(100 * muted / blocks, 2),
                "frames_dropped": int(dropped), "top_species_kept_pct": round(100 * kept_top / len(birds), 1)}
print(f"birds: {len(birds)} frames, {100*muted/blocks:.1f} % of blocks muted, {dropped} frames dropped, top species unchanged {100*kept_top/len(birds):.0f} %")
worst = sorted(per_file.items(), key=lambda kv: -kv[1][0] / kv[1][1])[:8]
for n, (m, t, d) in worst:
    print(f"   {100*m/t:5.1f} % muted, {d} dropped  {n[:70]}")
res["birds_worst"] = [{"file": n, "muted_pct": round(100 * m / t, 1), "dropped": int(d)} for n, (m, t, d) in worst]
json.dump(res, open("gate_results.json", "w"), indent=1)

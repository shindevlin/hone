"""Embed audio clips with Perch v2 int8 LiteRT (the exact model the Hone phone runs).

usage: embed.py OUT.npz LIST.txt   (LIST: one "label<TAB>session<TAB>path" per line)
Clips are resampled to 32 kHz mono. A clip <= 5 s is centred in a 5 s frame padded with
low-level noise taken from the clip's own quietest 50 ms (so the frame isn't digital silence);
longer clips are cut into 5 s frames (hop 2.5 s) and each frame is kept separately with the
clip index, so callers can choose per-frame or per-clip aggregation.
"""
import sys, numpy as np, soundfile as sf, soxr
from ai_edge_litert.interpreter import Interpreter

SR, FRAME, EMB = 32000, 160000, 1536
out_path, list_path = sys.argv[1], sys.argv[2]
it = Interpreter(model_path="/home/beastly/voiceprint-eval/model/perch_v2_8bit.tflite", num_threads=4)
it.allocate_tensors()
inp = it.get_input_details()[0]
outs = it.get_output_details()
emb_idx = [o["index"] for o in outs if list(o["shape"]) == [1, EMB]][0]
lab_idx = [o["index"] for o in outs if o["shape"][-1] == 14795][0]
import json, os
_names = json.load(open("/home/beastly/voiceprint-eval/model/class_names.json"))
_names = {v - 1: k for k, v in _names.items()}
TARGET = os.environ.get("TARGET")
tidx = [i for i, n in _names.items() if n == TARGET][0] if TARGET else None
_hum = [i for i, n in _names.items() if __import__("re").search(r"(?i)speech|speaking|voice|conversation|whisper|singing|shout|scream|laugh|giggle|chuckle|chatter|crowd|crying|babbl|cough|sneeze|^homo sapiens$|human", n)]

def frames(x):
    if len(x) <= FRAME:
        rng = np.random.default_rng(len(x))
        w = 1600
        if len(x) > 4 * w:
            e = [np.std(x[i:i + w]) for i in range(0, len(x) - w, w)]
            s = float(min(e))
        else:
            s = 1e-4
        f = rng.normal(0, s, FRAME).astype(np.float32)
        o = (FRAME - len(x)) // 2
        f[o:o + len(x)] = x
        return [f]
    hop = FRAME // 2
    return [x[i:i + FRAME] for i in range(0, len(x) - FRAME + 1, hop)]

rows = [l.rstrip("\n").split("\t") for l in open(list_path) if l.strip()]
E, L, S, C, P, TOP, CONF, HUM = [], [], [], [], [], [], [], []
for ci, (lab, sess, path) in enumerate(rows):
    try:
        x, sr = sf.read(path, dtype="float32", always_2d=True)
    except Exception as ex:
        print("skip", path, ex, file=sys.stderr); continue
    x = x.mean(axis=1)
    if sr != SR:
        x = soxr.resample(x, sr, SR).astype(np.float32)
    for f in frames(x):
        it.set_tensor(inp["index"], f[None, :].astype(np.float32))
        it.invoke()
        e = it.get_tensor(emb_idx)[0].copy()
        p = it.get_tensor(lab_idx)[0]
        E.append(e); L.append(lab); S.append(sess); C.append(ci); P.append(path)
        TOP.append(int(np.argmax(p)))
        CONF.append(float(p[tidx]) if tidx is not None else float(p.max()))
        HUM.append(float(p[_hum].max()))
    if ci % 100 == 0:
        print(ci, len(rows), flush=True)
np.savez_compressed(out_path, emb=np.array(E, np.float32), label=np.array(L), session=np.array(S),
                    clip=np.array(C), path=np.array(P), top=np.array(TOP), conf=np.array(CONF), human=np.array(HUM))
print("saved", out_path, len(E))

"""Hone voice check for library clips: the third check (Birderlands proposal hard-birds §5, Josh 2026-11-13: "use it").

The phone already deletes voices twice (the record bar and a check of the cleaned file). Before a clip is
published in the library, Birderlands' server sends it here and Hone runs a dedicated speech detector:
Silero VAD v5.1.2 (MIT licence, ONNX, models/silero_vad.onnx, sha256 2623a295...). If it hears speech
anywhere, the clip must stay private and the player is told (Birderlands enforces that).

- Audio is decoded and checked in RAM only, in a short-lived child process; nothing is written or logged.
- Only services on this machine can call it (POST /api/birds/voicecheck, never through Caddy).
- Rule (OURS): speech = Silero's score >= SPEECH_P for MIN_RUN chunks in a row (4 x 32 ms = 128 ms).
  Test set (13 Nov: 16 real bird recordings, words in 8 languages mixed at -6/-18/-30 dB): 3/16 clean birds held,
  speech found 14/16 loud, 6/16 quiet, 4/16 distant. Silero cannot hear faint words under birdsong, so this is a
  backstop on top of the phone's two checks, not a replacement; a stronger detector is the next step.
"""
import io
import json
import os
import subprocess
import sys
import threading

HERE = os.path.dirname(os.path.abspath(__file__))
MODEL = os.path.join(HERE, "models", "silero_vad.onnx")
MODEL_ID = "silero-vad-5.1.2"
SR, CHUNK, CTX = 16000, 512, 64
SPEECH_P = 0.2    # OURS: well under Silero's usual 0.5, because a missed voice is worse than a clip kept private
MIN_RUN = 4       # OURS: chunks in a row (128 ms); single spikes are usually birds
BLOCK_S = 5
MAX_S = 75        # library clips are <= 60 s; a little slack
_one = threading.Semaphore(1)  # one check at a time on this 1-CPU box


def decode(data):
    import av
    import numpy as np
    out, total = [], 0
    with av.open(io.BytesIO(data)) as c:
        rs = av.AudioResampler(format="flt", layout="mono", rate=SR)
        for frame in c.decode(c.streams.audio[0]):
            for f in rs.resample(frame):
                a = f.to_ndarray().reshape(-1)
                out.append(a)
                total += len(a)
            if total > MAX_S * SR:
                raise ValueError("clip longer than %d s" % MAX_S)
        for f in rs.resample(None):
            out.append(f.to_ndarray().reshape(-1))
    return np.concatenate(out).astype(np.float32) if out else np.zeros(0, np.float32)


def probabilities(pcm):
    import numpy as np
    import onnxruntime as ort
    so = ort.SessionOptions()
    so.intra_op_num_threads = 1
    so.inter_op_num_threads = 1
    s = ort.InferenceSession(MODEL, sess_options=so, providers=["CPUExecutionProvider"])
    state = np.zeros((2, 1, 128), np.float32)
    ctx = np.zeros((1, CTX), np.float32)
    sr = np.array(SR, np.int64)
    ps = []
    for i in range(0, len(pcm) - CHUNK + 1, CHUNK):
        x = np.concatenate([ctx, pcm[i:i + CHUNK][None, :]], axis=1)
        out, state = s.run(None, {"input": x, "state": state, "sr": sr})
        ctx = x[:, -CTX:]
        ps.append(float(out[0, 0]))
    return ps


def analyse(data, bar=SPEECH_P, run=MIN_RUN):
    pcm = decode(data)
    ps = probabilities(pcm)
    per_block = BLOCK_S * SR // CHUNK
    blocks, hit, n = {}, set(), 0
    for i, p in enumerate(ps):
        b = i // per_block
        blocks[b] = max(blocks.get(b, 0.0), p)
        n = n + 1 if p >= bar else 0
        if n >= run:
            hit.update({b, (i - run + 1) // per_block})
    speech = sorted(hit)
    return {"speech": bool(speech), "speech_blocks": [{"start_s": b * BLOCK_S, "p": round(blocks[b], 3)} for b in speech],
            "max_p": round(max(ps), 3) if ps else 0.0, "seconds": round(len(pcm) / SR, 2), "bar": bar, "min_run_ms": run * 32,
            "model": MODEL_ID}


def check_clip(data, timeout=90):
    """Runs the check in a child process (keeps the indexer's own memory small). Raises on failure:
    the caller must treat a failed check as 'not checked' and keep the clip private."""
    if not data or len(data) > 4 * 1024 * 1024:
        raise ValueError("clip must be 1 byte to 4 MB")
    with _one:
        r = subprocess.run([sys.executable, "-I", os.path.abspath(__file__)], input=data, capture_output=True, timeout=timeout)
    if r.returncode != 0:
        raise ValueError("voice check failed: " + r.stderr.decode(errors="replace").strip().splitlines()[-1][:160]
                         if r.stderr else "voice check failed")
    return json.loads(r.stdout)


if __name__ == "__main__":
    sys.stdout.write(json.dumps(analyse(sys.stdin.buffer.read())))

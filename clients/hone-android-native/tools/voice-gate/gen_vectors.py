"""Golden values for VoiceGateTest.kt from the Python reference (voice_gate.py)."""
import json, os, sys
import numpy as np
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import voice_gate as vg

SR = 32000
t = np.arange(5 * SR) / SR


def synth():
    # bird-like: 3 kHz tone, on for the first half of every 250 ms
    bird = 0.05 * np.sin(2 * np.pi * 3000 * t) * (((t * 4) % 1.0) < 0.5)
    # voice-like: 140 Hz harmonic stack, t in [2.0, 3.0)
    voice = np.zeros_like(t)
    on = (t >= 2.0) & (t < 3.0)
    for k in range(1, 9):
        voice += (0.1 / k) * np.sin(2 * np.pi * k * 140 * t)
    voice *= on
    return (bird + voice).astype(np.float32)


x = synth()
voiced, peak, edb = vg.voiced_frames(x)
mask, drop = vg.speech_mask(x)
hops = [0, 100, 210, 250, 290, 400, 496]
out = {
    "mask": [bool(m) for m in mask],
    "drop": drop,
    "voiced_count": int(voiced.sum()),
    "hops": hops,
    "peak": [float(peak[h]) for h in hops],
    "edb": [float(edb[h]) for h in hops],
}
print(json.dumps(out))
print("masked blocks:", [i for i, m in enumerate(mask) if m])

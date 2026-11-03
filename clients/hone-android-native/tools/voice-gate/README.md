# Hone voice gate: reference and test

`voice_gate.py` is the reference for `app/src/main/java/network/hone/app/sound/VoiceGate.kt`. Both must behave the same; `VoiceGateTest` checks the Kotlin side against values from `gen_vectors.py`.

What it does (sightings spec 2f, layer 5, option 2 "mute speech, keep the gaps"):
- It finds voiced human speech below 1 kHz (pitch 70–500 Hz) in 100 ms blocks. Continuous voice with no pause (singing, chanting) is caught too.
- It mutes those blocks, plus 0.3 s either side.
- It drops the whole 5 s frame if more than half of it is speech.
- Perch's human classes are a second gate on what is left.

## Run the test
Needs the Perch v2 8-bit LiteRT model (`PERCH_DIR`, default `~/perch`, with `perch_v2_8bit.tflite` and `assets/class_names.json`), `ai-edge-litert`, `numpy` and `soundfile`.

```
# speech: LibriSpeech dev-clean (CC BY 4.0) from https://www.openslr.org/12
# birds: openly licensed recordings (CC0 / PD / CC BY / CC BY-SA), e.g. from Wikimedia Commons
python fetch_birds.py data/birds 40
python gate_eval.py data/LibriSpeech/dev-clean data/birds 300
python gen_vectors.py            # golden values for VoiceGateTest.kt
```

Pass rule: speech blocks leaked must stay **under 1 %** in every condition (normal, quiet, under birdsong).

## Results (2026-11-03, grouchly)

| Condition | Speech blocks leaked |
|---|---|
| Normal (−26 dBFS) | 0 / 11,826 = 0.00 % |
| Quiet (−40 dBFS) | 0 / 11,826 = 0.00 % |
| Under birdsong (speech −32, bird −26 dBFS) | 15 / 11,826 = 0.13 % |

Birds (40 frames, skewed to low-voiced species on purpose):
- 33.8 % of blocks muted;
- top species unchanged in 60 % of frames (the old drop-whole-frame gate: 22 %);
- most muted: doves, owls, nightjars, which call in the human pitch range.

For comparison, Perch's human classes alone let through 38 % (normal), 65 % (quiet) and 100 % (under birdsong) of speech frames.

## Known gaps
- English read speech only. Add multilingual speech (e.g. FLEURS, CC BY 4.0), children, shouting, phone-distance recordings.
- **Whispers carry no pitch**, so this detector cannot see them.
- Bird false-mute set is small. Grow it with license-checked recordings; keep the license manifest.
- Steady engine hum in 70–500 Hz can be muted as voice. That is the safe side, but it biases noise levels in traffic. Measure it.

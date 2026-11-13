# Models (not in git)

`silero_vad.onnx`: Silero VAD v5.1.2, MIT licence, used by `hone_voicecheck.py` (library voice check).

    curl -sSL -o models/silero_vad.onnx https://raw.githubusercontent.com/snakers4/silero-vad/v5.1.2/src/silero_vad/data/silero_vad.onnx
    sha256: 2623a2953f6ff3d2c1e61740c6cdb7168133479b267dfef114a4a3cc5bdd788f (2,327,524 bytes)

Python: `pip install --only-binary :all: onnxruntime av numpy`.

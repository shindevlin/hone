#!/usr/bin/env bash
set -u
cd ~/voiceprint-eval
m=https://huggingface.co/Arm/perch-v2-int8-litert/resolve/main
[ -f model/perch_v2_8bit.tflite ] || curl -fsSL -o model/perch_v2_8bit.tflite $m/perch_v2_8bit.tflite
[ -f model/class_names.json ] || curl -fsSL -o model/class_names.json $m/assets/class_names.json
echo "9ccbfc424bf35efd1d1f4604a3339c2bb9e03b1c7f2ca7b8ccea7f0a026ceb0c  model/perch_v2_8bit.tflite" | sha256sum -c
z=https://zenodo.org/records/1413495/files
for f in csv.zip README.txt littleowl-fg.zip pipit-fg.zip chiffchaff-fg.zip; do
  [ -f data/stowell/$f ] || { mkdir -p data/stowell; curl -fsSL -o data/stowell/$f.part "$z/$f?download=1" && mv data/stowell/$f.part data/stowell/$f; }
  echo done $f
done
mkdir -p data/lostanlen
[ -f data/lostanlen/dataset.zip ] || { curl -fsSL -o data/lostanlen/dataset.zip.part "https://zenodo.org/records/15054394/files/dataset.zip?download=1" && mv data/lostanlen/dataset.zip.part data/lostanlen/dataset.zip; }
curl -fsSL -o data/lostanlen/README.txt "https://zenodo.org/records/15054394/files/README.txt?download=1"
echo ALLDONE

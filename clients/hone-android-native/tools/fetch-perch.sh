#!/usr/bin/env bash
# Fetch Perch v2 (8-bit LiteRT, Apache-2.0) into app assets and check hashes.
# Source: https://huggingface.co/Arm/perch-v2-int8-litert (derived from Google DeepMind Perch v2).
# The model (~105 MB) is not committed to git.
set -euo pipefail
cd "$(dirname "$0")/.."
dst=app/src/main/assets/perch
mkdir -p "$dst"
base=https://huggingface.co/Arm/perch-v2-int8-litert/resolve/main
fetch() { [ -f "$dst/$(basename "$1")" ] || curl -fsSL -o "$dst/$(basename "$1")" "$base/$1"; }
fetch perch_v2_8bit.tflite
fetch assets/class_names.json
fetch README.md
check() { echo "$1  $dst/$2" | sha256sum -c --quiet; }
check 9ccbfc424bf35efd1d1f4604a3339c2bb9e03b1c7f2ca7b8ccea7f0a026ceb0c perch_v2_8bit.tflite
check 51d8f01ad879eaa8994de0490961ceab6cdca84b985384a397e7b83fd1e07c03 class_names.json
cat > "$dst/NOTICE.txt" <<'EOF'
Perch v2 bird vocalization classifier, Copyright Google LLC, Apache License 2.0.
8-bit LiteRT packaging by Arm (huggingface.co/Arm/perch-v2-int8-litert), Apache License 2.0.
Used unmodified by HONE.
EOF
echo "perch assets ok"

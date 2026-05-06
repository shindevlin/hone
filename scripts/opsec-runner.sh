#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

OUT_DIR="${1:-reports/opsec/evidence-manual}"
mkdir -p "$OUT_DIR"

echo "[opsec] root=$ROOT_DIR"
echo "[opsec] out=$OUT_DIR"

# Gitleaks with explicit repo config.
set +e
docker run --rm \
  -v "$ROOT_DIR:/repo" \
  zricethezav/gitleaks:latest detect \
  --config=/repo/.gitleaks.toml \
  --source=/repo \
  --report-format=json \
  --report-path="/repo/$OUT_DIR/gitleaks.json" \
  --redact > "$OUT_DIR/gitleaks.stdout" 2> "$OUT_DIR/gitleaks.stderr"
printf "%s" $? > "$OUT_DIR/gitleaks.exit"
set -e

# Trivy with explicit ignore file.
set +e
docker run --rm \
  -v "$ROOT_DIR:/repo" \
  aquasec/trivy:0.56.2 fs \
  --severity HIGH,CRITICAL \
  --ignore-unfixed \
  --ignorefile /repo/.trivyignore \
  --format json \
  /repo > "$OUT_DIR/trivy.json" 2> "$OUT_DIR/trivy.stderr"
printf "%s" $? > "$OUT_DIR/trivy.exit"
set -e

# Compact summary.
OUT_DIR_ENV="$OUT_DIR" node <<'NODE'
const fs = require('fs');
const out = process.env.OUT_DIR_ENV;
const t = JSON.parse(fs.readFileSync(`${out}/trivy.json`, 'utf8'));
let high = 0, critical = 0, total = 0;
for (const r of (t.Results || [])) {
  for (const v of (r.Vulnerabilities || [])) {
    total++;
    if (v.Severity === 'HIGH') high++;
    if (v.Severity === 'CRITICAL') critical++;
  }
}
let gitleaks = 0;
if (fs.existsSync(`${out}/gitleaks.json`)) {
  const g = JSON.parse(fs.readFileSync(`${out}/gitleaks.json`, 'utf8'));
  gitleaks = Array.isArray(g) ? g.length : (g.findings || []).length;
}
console.log(JSON.stringify({ trivy: { total, high, critical }, gitleaks }, null, 2));
NODE

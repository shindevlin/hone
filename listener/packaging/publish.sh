#!/usr/bin/env bash
# Assemble a release directory: version.json (+ .sig, ed25519 by the HONE listener release key
# ~/.config/hone-release/listener-ed25519.pem) over every file in it.
#   publish.sh <release-dir> <version> [verifier-binary]
# The directory is then copied as-is to ferryvps:/var/www/hone-app/listener/
#   (served at https://honemesh.net/app/listener/).
set -euo pipefail
D=$1 V=$2 BIN=${3:-}
KEY="$HOME/.config/hone-release/listener-ed25519.pem"
PIN=27ef54588674c3391e660056d873b89136e75125b5a433de0747191baa94a10d
GOT=$(openssl pkey -in "$KEY" -pubout -outform DER | tail -c 32 | od -An -tx1 | tr -d ' \n')
[ "$GOT" = "$PIN" ] || { echo "release key does not match the pinned public key" >&2; exit 1; }
python3 - "$D" "$V" <<'PY'
import hashlib, json, os, sys, datetime
d, v = sys.argv[1], sys.argv[2]
files = {}
for n in sorted(os.listdir(d)):
    p = os.path.join(d, n)
    if not os.path.isfile(p) or n in ("version.json", "version.json.sig") or n.startswith("."):
        continue
    files[n] = {"sha256": hashlib.sha256(open(p, "rb").read()).hexdigest(), "size": os.path.getsize(p)}
m = {"name": "hone-listener", "version": v, "released": datetime.date.today().isoformat(), "files": files,
     "local": {"systemd_unit": "hone-listener", "windows_task": "HONE Listener", "launchd_label": "net.honemesh.listener", "status_port": 47811}}
open(os.path.join(d, "version.json"), "w").write(json.dumps(m, indent=1, sort_keys=True) + "\n")
PY
T=$(mktemp); trap 'rm -f "$T"' EXIT
openssl pkeyutl -sign -inkey "$KEY" -rawin -in "$D/version.json" -out "$T"
od -An -tx1 "$T" | tr -d ' \n' > "$D/version.json.sig"; echo >> "$D/version.json.sig"
openssl pkey -in "$KEY" -pubout -out "$T.pub"
openssl pkeyutl -verify -pubin -inkey "$T.pub" -rawin -in "$D/version.json" -sigfile "$T" >/dev/null && echo "version.json signed (openssl verify ok)"
rm -f "$T.pub"
[ -n "$BIN" ] && "$BIN" verify-release "$D/version.json" "$D/version.json.sig" "$D"/hone-listener* "$D"/install.*
ls -la "$D"

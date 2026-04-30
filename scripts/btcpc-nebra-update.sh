#!/bin/bash
# btcpc-nebra-update.sh — self-update btcpc-node on Nebra Pi from GitHub releases.
# Runs via systemd timer (daily). Checks the latest release, downloads the
# aarch64 binary if newer than the installed version, verifies it, installs,
# and restarts the service.
set -euo pipefail

BINARY=/usr/local/bin/btcpc-node
REPO="btcpc-network/btcpc"
ASSET_NAME="btcpc-node-aarch64-linux"
SERVICE="btcpc-node"
TMP=/tmp/btcpc-node-update

log() { echo "[btcpc-update] $(date -u +%Y-%m-%dT%H:%M:%SZ) $*"; }

# Current version
CURRENT_VER=""
if [ -f "$BINARY" ]; then
    CURRENT_VER=$("$BINARY" --version 2>/dev/null | awk '{print $NF}' || true)
fi

# Latest release from GitHub
LATEST_JSON=$(curl -fsSL --max-time 15 \
    "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null) || {
    log "GitHub API unreachable — skipping update"
    exit 0
}

LATEST_VER=$(echo "$LATEST_JSON" | python3 -c "import sys,json; print(json.load(sys.stdin)['tag_name'].lstrip('v'))" 2>/dev/null || true)
if [ -z "$LATEST_VER" ]; then
    log "Could not parse latest release version — skipping"
    exit 0
fi

if [ "$CURRENT_VER" = "$LATEST_VER" ]; then
    log "Already on $LATEST_VER — no update needed"
    exit 0
fi

log "Update available: $CURRENT_VER → $LATEST_VER"

ASSET_URL=$(echo "$LATEST_JSON" | python3 -c "
import sys, json
d = json.load(sys.stdin)
for a in d.get('assets', []):
    if '$ASSET_NAME' in a['name']:
        print(a['browser_download_url'])
        break
" 2>/dev/null || true)

if [ -z "$ASSET_URL" ]; then
    log "No $ASSET_NAME asset in release $LATEST_VER — skipping"
    exit 0
fi

mkdir -p "$TMP"
DOWNLOADED="$TMP/btcpc-node"

log "Downloading $ASSET_URL"
curl -fsSL --max-time 120 -o "$DOWNLOADED" "$ASSET_URL"
chmod +x "$DOWNLOADED"

# Sanity check: must be an ELF binary for aarch64
if ! file "$DOWNLOADED" | grep -q "aarch64"; then
    log "Downloaded binary is not aarch64 — aborting"
    rm -f "$DOWNLOADED"
    exit 1
fi

log "Installing btcpc-node v$LATEST_VER"
sudo install -m 755 "$DOWNLOADED" "$BINARY"
rm -rf "$TMP"

log "Restarting $SERVICE"
sudo systemctl restart "$SERVICE"
log "Update complete: now running v$LATEST_VER"

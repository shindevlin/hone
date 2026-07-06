#!/bin/bash
# hone-nebra-update.sh — self-update hone-node on Nebra Pi from GitHub releases.
# Runs via crontab (daily). Checks latest stable release, downloads the
# aarch64 binary if newer than installed, and hot-swaps the service.
set -euo pipefail

BINARY_DIR=/home/pi/hone-node
BINARY=$BINARY_DIR/hone-node
VERSION_FILE=$BINARY_DIR/.hone-node-version
REPO="shindevlin/btcpc"
ASSET_NAME="hone-node-aarch64-linux"
SERVICE="hone-node"
TMP=/tmp/hone-node-update

log() { echo "[hone-update] $(date -u +%Y-%m-%dT%H:%M:%SZ) $*"; }

# Current installed version
CURRENT_VER=""
if [ -f "$VERSION_FILE" ]; then
    CURRENT_VER=$(cat "$VERSION_FILE")
fi

# Latest stable release from GitHub
LATEST_JSON=$(curl -fsSL --max-time 15 \
    "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null) || {
    log "GitHub API unreachable — skipping update"
    exit 0
}

LATEST_VER=$(echo "$LATEST_JSON" | python3 -c \
    "import sys,json; print(json.load(sys.stdin)['tag_name'])" 2>/dev/null || true)

if [ -z "$LATEST_VER" ]; then
    log "Could not parse latest release tag — skipping"
    exit 0
fi

if [ "$LATEST_VER" = "$CURRENT_VER" ]; then
    log "Already at $LATEST_VER — no update needed"
    exit 0
fi

log "Update available: ${CURRENT_VER:-none} -> $LATEST_VER"

# Find download URL for the aarch64 asset
ASSET_URL=$(echo "$LATEST_JSON" | python3 -c "
import sys, json
data = json.load(sys.stdin)
for a in data.get('assets', []):
    if '$ASSET_NAME' in a['name']:
        print(a['browser_download_url'])
        break
" 2>/dev/null || true)

if [ -z "$ASSET_URL" ]; then
    log "Asset '$ASSET_NAME' not found in release $LATEST_VER — skipping"
    exit 0
fi

log "Downloading $ASSET_URL"
mkdir -p "$TMP"
curl -fsSL --max-time 120 -o "$TMP/hone-node-new" "$ASSET_URL"
chmod +x "$TMP/hone-node-new"

# Sanity check: binary must be ELF aarch64
if ! file "$TMP/hone-node-new" | grep -q "aarch64"; then
    log "Downloaded file is not an aarch64 ELF binary — aborting"
    rm -f "$TMP/hone-node-new"
    exit 1
fi

log "Swapping binary and restarting service"
systemctl --user stop "$SERVICE" || true
mv "$TMP/hone-node-new" "$BINARY"
echo "$LATEST_VER" > "$VERSION_FILE"
systemctl --user start "$SERVICE"

log "Updated to $LATEST_VER — done"

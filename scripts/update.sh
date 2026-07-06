#!/usr/bin/env bash
# Download the latest hone-node release binary and restart the service.
# Safe to run manually or via hone-update.timer.
#
# Usage:  sudo bash update.sh

set -euo pipefail

GITHUB_REPO="shindevlin/btcpc"
ASSET="hone-node-x86_64-linux"
BINARY="/usr/local/bin/hone-node"
SERVICE="hone-node"

# ── Fetch latest release tag ──────────────────────────────────────────────────
LATEST=$(curl -fsSL "https://api.github.com/repos/${GITHUB_REPO}/releases" \
    | grep -m1 '"tag_name"' \
    | grep 'node-v' \
    | sed 's/.*"tag_name": "\(.*\)".*/\1/')

if [ -z "$LATEST" ]; then
    echo "hone-update: could not determine latest release tag — aborting"
    exit 1
fi

# ── Compare with running binary ───────────────────────────────────────────────
CURRENT_TAG=$(cat /var/lib/hone/.release-tag 2>/dev/null || echo "none")
if [ "$CURRENT_TAG" = "$LATEST" ]; then
    echo "hone-update: already on ${LATEST}"
    exit 0
fi

echo "hone-update: ${CURRENT_TAG} -> ${LATEST}"

# ── Download ──────────────────────────────────────────────────────────────────
DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/${LATEST}/${ASSET}"
curl -fsSL "$DOWNLOAD_URL" -o "${BINARY}.tmp"
chmod +x "${BINARY}.tmp"
mv "${BINARY}.tmp" "$BINARY"
echo "$LATEST" > /var/lib/hone/.release-tag
echo "hone-update: binary updated to ${LATEST}"

# ── Restart ───────────────────────────────────────────────────────────────────
if systemctl is-active --quiet "$SERVICE"; then
    systemctl restart "$SERVICE"
    echo "hone-update: ${SERVICE} restarted"
fi

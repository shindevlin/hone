#!/usr/bin/env bash
# Pull latest code, rebuild, and restart btcpc-node if there are new commits.
# Safe to run manually or via the btcpc-update.timer systemd unit.
#
# Usage:  sudo bash /opt/btcpc/scripts/update.sh

set -euo pipefail

INSTALL_DIR="/opt/btcpc"
BINARY="/usr/local/bin/btcpc-node"
SERVICE="btcpc-node"

cd "$INSTALL_DIR"

# ── Check for upstream changes ────────────────────────────────────────────────
git fetch origin stable --quiet

LOCAL=$(git rev-parse HEAD)
REMOTE=$(git rev-parse origin/stable)

if [ "$LOCAL" = "$REMOTE" ]; then
    echo "btcpc-update: already up to date ($LOCAL)"
    exit 0
fi

echo "btcpc-update: new commits — updating $LOCAL -> $REMOTE"
git pull --ff-only origin stable

# ── Rebuild ───────────────────────────────────────────────────────────────────
echo "btcpc-update: building"
source "$HOME/.cargo/env" 2>/dev/null || true
cd "$INSTALL_DIR/rust/btcpc-node"
cargo build --release --quiet

# ── Deploy ────────────────────────────────────────────────────────────────────
cp target/release/btcpc-node "$BINARY"
echo "btcpc-update: binary updated"

# ── Restart node ──────────────────────────────────────────────────────────────
if systemctl is-active --quiet "$SERVICE"; then
    systemctl restart "$SERVICE"
    echo "btcpc-update: $SERVICE restarted"
else
    echo "btcpc-update: $SERVICE is not running — skipping restart"
fi

echo "btcpc-update: done ($REMOTE)"

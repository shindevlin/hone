#!/usr/bin/env bash
# First-time BTCPC node setup on a new machine.
# Run as root or with sudo.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/shindevlin/btcpc/stable/scripts/install.sh | sudo bash

set -euo pipefail

GITHUB_REPO="shindevlin/btcpc"
RELEASE_TAG="node-v1.0.0"
ASSET="btcpc-node-linux-x86_64"
BINARY="/usr/local/bin/btcpc-node"
SCRIPTS_BASE="https://raw.githubusercontent.com/${GITHUB_REPO}/stable/scripts"

echo "==> Installing BTCPC node ${RELEASE_TAG}"

# ── Download binary ───────────────────────────────────────────────────────────
DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/${RELEASE_TAG}/${ASSET}"
echo "==> Downloading binary from ${DOWNLOAD_URL}"
curl -fsSL "$DOWNLOAD_URL" -o "$BINARY"
chmod +x "$BINARY"
echo "==> Installed: $BINARY"

# ── Data directory ────────────────────────────────────────────────────────────
mkdir -p /var/lib/btcpc
id btcpc &>/dev/null || useradd --system --no-create-home --home /var/lib/btcpc btcpc
chown btcpc:btcpc /var/lib/btcpc

# ── Systemd units ─────────────────────────────────────────────────────────────
echo "==> Installing systemd units"
curl -fsSL "${SCRIPTS_BASE}/btcpc-node.service"   -o /etc/systemd/system/btcpc-node.service
curl -fsSL "${SCRIPTS_BASE}/btcpc-update.service" -o /etc/systemd/system/btcpc-update.service
curl -fsSL "${SCRIPTS_BASE}/btcpc-update.timer"   -o /etc/systemd/system/btcpc-update.timer
systemctl daemon-reload

# ── Enable auto-update timer ──────────────────────────────────────────────────
systemctl enable --now btcpc-update.timer
echo "==> Auto-updates enabled"

echo ""
echo "==> Done. Edit /etc/systemd/system/btcpc-node.service then:"
echo "    systemctl enable --now btcpc-node"
echo ""
echo "    Required env vars to set in the service file:"
echo "      BTCPC_ACCOUNT          — your account name"
echo "      BTCPC_NODE_ID          — unique node label"
echo "      BTCPC_GENESIS_TIMESTAMP=1777590000000"
echo "      BTCPC_CHAIN_ID=btcpc-1"
echo "      BTCPC_BOOTSTRAP_PEERS  — (optional, auto-fetched from Hive)"
echo ""

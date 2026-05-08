#!/usr/bin/env bash
# First-time BTCPC node setup on a new machine.
# Run as root or with sudo.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/shindevlin/btcpc/stable/scripts/install.sh | sudo bash

set -euo pipefail

GITHUB_REPO="shindevlin/btcpc"
RELEASE_TAG="node-v1.0.0"
ASSET_NODE="btcpc-node-linux-x86_64"
ASSET_CLI="btcpc-linux-x86_64"
BINARY_NODE="/usr/local/bin/btcpc-node"
BINARY_CLI="/usr/local/bin/btcpc"
SCRIPTS_BASE="https://raw.githubusercontent.com/${GITHUB_REPO}/stable/scripts"

echo "==> Installing BTCPC node and CLI ${RELEASE_TAG}"

# ── Download node binary ──────────────────────────────────────────────────────
DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/${RELEASE_TAG}/${ASSET_NODE}"
echo "==> Downloading node binary from ${DOWNLOAD_URL}"
curl -fsSL "$DOWNLOAD_URL" -o "$BINARY_NODE"
chmod +x "$BINARY_NODE"
echo "==> Installed: $BINARY_NODE"

# ── Download CLI binary ───────────────────────────────────────────────────────
CLI_URL="https://github.com/${GITHUB_REPO}/releases/download/${RELEASE_TAG}/${ASSET_CLI}"
echo "==> Downloading btcpc CLI from ${CLI_URL}"
curl -fsSL "$CLI_URL" -o "$BINARY_CLI"
chmod +x "$BINARY_CLI"
echo "==> Installed: $BINARY_CLI"

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
echo "==> Done. Next steps:"
echo ""
echo "  1. Create your account and log in:"
echo "       btcpc account create yourname"
echo "       btcpc login"
echo ""
echo "  2. Edit the node service file:"
echo "       nano /etc/systemd/system/btcpc-node.service"
echo "     Required env vars:"
echo "       BTCPC_ACCOUNT          — your account name"
echo "       BTCPC_NODE_ID          — unique node label"
echo "       BTCPC_GENESIS_TIMESTAMP=1777633200000"
echo "       BTCPC_CHAIN_ID=btcpc-1"
echo "       BTCPC_BOOTSTRAP_PEERS  — (optional, auto-fetched from Hive)"
echo ""
echo "  3. Start the node:"
echo "       systemctl enable --now btcpc-node"
echo ""
echo "  4. Publish a git repo:"
echo "       cd my-project"
echo "       btcpc repo init my-project"
echo "       git push -u origin main"
echo ""

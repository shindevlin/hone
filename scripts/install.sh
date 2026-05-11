#!/usr/bin/env bash
# First-time BTCPC node setup on a new machine.
# Run as root or with sudo.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/shindevlin/btcpc/stable/scripts/install.sh | sudo bash

set -euo pipefail

GITHUB_REPO="shindevlin/btcpc"
RELEASE_TAG="node-v1.1.0"
ARCH="$(uname -m)"
case "$ARCH" in
  aarch64|arm64) SUFFIX="linux-aarch64" ;;
  *)             SUFFIX="linux-x86_64"  ;;
esac
ASSET_NODE="btcpc-node-${SUFFIX}"
ASSET_CLI="btcpc-${SUFFIX}"
BINARY_NODE="/usr/local/bin/btcpc-node"
BINARY_CLI="/usr/local/bin/btcpc"
RELEASE_BASE="https://github.com/${GITHUB_REPO}/releases/download/${RELEASE_TAG}"
SCRIPTS_BASE="https://raw.githubusercontent.com/${GITHUB_REPO}/stable/scripts"
RAW_BASE="https://raw.githubusercontent.com/${GITHUB_REPO}/stable"

echo "==> Installing BTCPC node and CLI ${RELEASE_TAG} (${SUFFIX})"

# ── Download node binary ──────────────────────────────────────────────────────
echo "==> Downloading node binary"
curl -fsSL "${RELEASE_BASE}/${ASSET_NODE}" -o "$BINARY_NODE"
chmod +x "$BINARY_NODE"
echo "==> Installed: $BINARY_NODE"

# ── Download CLI binary ───────────────────────────────────────────────────────
echo "==> Downloading btcpc CLI"
curl -fsSL "${RELEASE_BASE}/${ASSET_CLI}" -o "$BINARY_CLI"
chmod +x "$BINARY_CLI"
echo "==> Installed: $BINARY_CLI"

# ── Mainnet data directory + user ────────────────────────────────────────────
mkdir -p /var/lib/btcpc
id btcpc &>/dev/null || useradd --system --no-create-home --home /var/lib/btcpc btcpc
chown btcpc:btcpc /var/lib/btcpc

# ── Testnet data directory + genesis ────────────────────────────────────────
mkdir -p /var/lib/btcpc-testnet
chown btcpc:btcpc /var/lib/btcpc-testnet
echo "==> Downloading testnet genesis"
curl -fsSL "${RAW_BASE}/rust/btcpc-node/testnet-genesis.json" \
    -o /var/lib/btcpc-testnet/genesis.json
chown btcpc:btcpc /var/lib/btcpc-testnet/genesis.json

# ── Systemd units ─────────────────────────────────────────────────────────────
echo "==> Installing systemd units"
curl -fsSL "${SCRIPTS_BASE}/btcpc-node.service"     -o /etc/systemd/system/btcpc-node.service
curl -fsSL "${SCRIPTS_BASE}/btcpc-testnet.service"  -o /etc/systemd/system/btcpc-testnet.service
curl -fsSL "${SCRIPTS_BASE}/btcpc-update.service"   -o /etc/systemd/system/btcpc-update.service
curl -fsSL "${SCRIPTS_BASE}/btcpc-update.timer"     -o /etc/systemd/system/btcpc-update.timer
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
echo "  2. Set your account in both service files:"
echo "       nano /etc/systemd/system/btcpc-node.service"
echo "       nano /etc/systemd/system/btcpc-testnet.service"
echo "     Required env vars in each:"
echo "       BTCPC_ACCOUNT   — your account name"
echo "       BTCPC_NODE_ID   — unique node label"
echo ""
echo "  3. Start both mainnet and testnet:"
echo "       systemctl enable --now btcpc-node btcpc-testnet"
echo ""
echo "  4. Check status:"
echo "       systemctl status btcpc-node btcpc-testnet"
echo "       curl http://localhost:4242/api/node/info   # mainnet"
echo "       curl http://localhost:4343/api/node/info   # testnet"
echo ""

#!/usr/bin/env bash
# First-time HONE node setup on a new machine.
# Run as root or with sudo.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/shindevlin/hone/stable/scripts/install.sh | sudo bash

set -euo pipefail

GITHUB_REPO="shindevlin/hone"
RELEASE_TAG="node-v1.1.0"
ARCH="$(uname -m)"
case "$ARCH" in
  aarch64|arm64) SUFFIX="aarch64-linux" ;;
  *)             SUFFIX="x86_64-linux"  ;;
esac
ASSET_NODE="hone-node-${SUFFIX}"
ASSET_CLI="hone-${SUFFIX}"
BINARY_NODE="/usr/local/bin/hone-node"
BINARY_CLI="/usr/local/bin/hone"
RELEASE_BASE="https://github.com/${GITHUB_REPO}/releases/download/${RELEASE_TAG}"
SCRIPTS_BASE="https://raw.githubusercontent.com/${GITHUB_REPO}/stable/scripts"
RAW_BASE="https://raw.githubusercontent.com/${GITHUB_REPO}/stable"

echo "==> Installing HONE node and CLI ${RELEASE_TAG} (${SUFFIX})"

# ── Download node binary ──────────────────────────────────────────────────────
echo "==> Downloading node binary"
curl -fsSL "${RELEASE_BASE}/${ASSET_NODE}" -o "$BINARY_NODE"
chmod +x "$BINARY_NODE"
echo "==> Installed: $BINARY_NODE"

# ── Download CLI binary ───────────────────────────────────────────────────────
echo "==> Downloading hone CLI"
curl -fsSL "${RELEASE_BASE}/${ASSET_CLI}" -o "$BINARY_CLI"
chmod +x "$BINARY_CLI"
echo "==> Installed: $BINARY_CLI"

# ── Mainnet data directory + user ────────────────────────────────────────────
mkdir -p /var/lib/hone
id hone &>/dev/null || useradd --system --no-create-home --home /var/lib/hone hone
chown hone:hone /var/lib/hone

# ── Testnet data directory + genesis ────────────────────────────────────────
mkdir -p /var/lib/hone-testnet
chown hone:hone /var/lib/hone-testnet
echo "==> Downloading testnet genesis"
curl -fsSL "${RAW_BASE}/rust/hone-node/testnet-genesis.json" \
    -o /var/lib/hone-testnet/genesis.json
chown hone:hone /var/lib/hone-testnet/genesis.json

# ── Systemd units ─────────────────────────────────────────────────────────────
echo "==> Installing systemd units"
curl -fsSL "${SCRIPTS_BASE}/hone-node.service"     -o /etc/systemd/system/hone-node.service
curl -fsSL "${SCRIPTS_BASE}/hone-testnet.service"  -o /etc/systemd/system/hone-testnet.service
curl -fsSL "${SCRIPTS_BASE}/hone-update.service"   -o /etc/systemd/system/hone-update.service
curl -fsSL "${SCRIPTS_BASE}/hone-update.timer"     -o /etc/systemd/system/hone-update.timer
systemctl daemon-reload

# ── Enable auto-update timer ──────────────────────────────────────────────────
systemctl enable --now hone-update.timer
echo "==> Auto-updates enabled"

echo ""
echo "==> Done. Next steps:"
echo ""
echo "  1. Create your account and log in:"
echo "       hone account create yourname"
echo "       hone login"
echo ""
echo "  2. Set your account in both service files:"
echo "       nano /etc/systemd/system/hone-node.service"
echo "       nano /etc/systemd/system/hone-testnet.service"
echo "     Required env vars in each:"
echo "       HONE_ACCOUNT   — your account name"
echo "       HONE_NODE_ID   — unique node label"
echo ""
echo "  3. Start both mainnet and testnet:"
echo "       systemctl enable --now hone-node hone-testnet"
echo ""
echo "  4. Check status:"
echo "       systemctl status hone-node hone-testnet"
echo "       curl http://localhost:4242/api/node/info   # mainnet"
echo "       curl http://localhost:4343/api/node/info   # testnet"
echo ""

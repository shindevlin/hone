#!/usr/bin/env bash
# First-time BTCPC node setup on a new machine.
# Run as root or with sudo.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/shindevlin/btcpc/main/scripts/install.sh | sudo bash
#   -- or --
#   sudo bash scripts/install.sh

set -euo pipefail

REPO="https://github.com/shindevlin/btcpc.git"
INSTALL_DIR="/opt/btcpc"
BINARY="/usr/local/bin/btcpc-node"
SERVICE_SRC="$INSTALL_DIR/rust/btcpc-node/btcpc-node.service"

echo "==> Installing BTCPC node"

# ── Dependencies ──────────────────────────────────────────────────────────────
if ! command -v cargo &>/dev/null; then
    echo "==> Installing Rust toolchain"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    source "$HOME/.cargo/env"
fi

if ! command -v git &>/dev/null; then
    apt-get update -qq && apt-get install -y -qq git
fi

# ── Clone or update ───────────────────────────────────────────────────────────
if [ -d "$INSTALL_DIR/.git" ]; then
    echo "==> Updating existing clone at $INSTALL_DIR"
    git -C "$INSTALL_DIR" pull --ff-only
else
    echo "==> Cloning into $INSTALL_DIR"
    git clone "$REPO" "$INSTALL_DIR"
fi

# ── Build ─────────────────────────────────────────────────────────────────────
echo "==> Building btcpc-node (release)"
cd "$INSTALL_DIR/rust/btcpc-node"
cargo build --release 2>&1
cp target/release/btcpc-node "$BINARY"
echo "==> Installed binary: $BINARY ($(btcpc-node --version 2>/dev/null || echo 'ok'))"

# ── Data directory ────────────────────────────────────────────────────────────
mkdir -p /var/lib/btcpc
id btcpc &>/dev/null || useradd --system --no-create-home --home /var/lib/btcpc btcpc
chown btcpc:btcpc /var/lib/btcpc

# ── Auto-update timer ─────────────────────────────────────────────────────────
cp "$INSTALL_DIR/scripts/btcpc-update.service" /etc/systemd/system/
cp "$INSTALL_DIR/scripts/btcpc-update.timer"   /etc/systemd/system/
systemctl daemon-reload

echo ""
echo "==> Done. Next steps:"
echo "    1. Edit /etc/systemd/system/btcpc-node.service — fill in env vars"
echo "       (BTCPC_ACCOUNT, BTCPC_NODE_ID, BTCPC_GENESIS_TIMESTAMP, etc.)"
echo "    2. systemctl enable --now btcpc-node"
echo "    3. To enable auto-updates: systemctl enable --now btcpc-update.timer"
echo ""

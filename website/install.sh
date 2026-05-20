#!/usr/bin/env bash
# BTCPC Node Installer
# Usage: curl -fsSL https://btcpc.net/install.sh | bash
set -e

ORANGE='\033[38;5;208m'
GREEN='\033[0;32m'
RED='\033[0;31m'
RESET='\033[0m'

say()  { printf "${ORANGE}[btcpc]${RESET} %s\n" "$1"; }
ok()   { printf "${GREEN}[ok]${RESET} %s\n" "$1"; }
err()  { printf "${RED}[error]${RESET} %s\n" "$1" >&2; exit 1; }

cat <<'BANNER'

  ██████╗ ████████╗ ██████╗██████╗  ██████╗
  ██╔══██╗╚══██╔══╝██╔════╝██╔══██╗██╔════╝
  ██████╔╝   ██║   ██║     ██████╔╝██║
  ██╔══██╗   ██║   ██║     ██╔═══╝ ██║
  ██████╔╝   ██║   ╚██████╗██║     ╚██████╗
  ╚═════╝    ╚═╝    ╚═════╝╚═╝      ╚═════╝

  Bitcoin Proof of Compute — Free to run, free forever.

BANNER

# ── Detect OS / arch ──────────────────────────────────────────────────────────
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux*)  PLATFORM="linux" ;;
  Darwin*) PLATFORM="mac"   ;;
  CYGWIN*|MINGW*|MSYS*) err "Use the Windows installer: https://btcpc.net/install" ;;
  *) err "Unsupported OS: $OS" ;;
esac

case "$ARCH" in
  x86_64|amd64)   ARCH_TAG="amd64" ;;
  aarch64|arm64)  ARCH_TAG="arm64" ;;
  *) ARCH_TAG="amd64" ;;
esac

BIN_NAME="btcpc-node-${PLATFORM}-${ARCH_TAG}"
DOWNLOAD_URL="https://btcpc.net/downloads/${BIN_NAME}"
INSTALL_BIN="/usr/local/bin/btcpc-node"
DATA_DIR="${BTCPC_DATA_DIR:-$HOME/.btcpc}"

# ── Ask account name ──────────────────────────────────────────────────────────
if [ -z "${BTCPC_ACCOUNT:-}" ]; then
  printf "${ORANGE}[btcpc]${RESET} Enter your BTCPC username (letters, numbers, hyphens): "
  read -r BTCPC_ACCOUNT
fi
[ -z "$BTCPC_ACCOUNT" ] && err "Account name required."

# ── Role selection ────────────────────────────────────────────────────────────
if [ -z "${BTCPC_ROLE_CHOICE:-}" ]; then
  echo ""
  say "Choose your node role:"
  echo "  1) Clock only      — lightweight, any machine, no GPU needed"
  echo "  2) Clock + Miner   — runs Ollama inference (default, GPU recommended)"
  echo "  3) Full node       — clock + miner + storage"
  printf "${ORANGE}[btcpc]${RESET} Choice [1/2/3, default=2]: "
  read -r BTCPC_ROLE_CHOICE
fi

case "${BTCPC_ROLE_CHOICE:-2}" in
  1) BTCPC_CLOCK=true;  BTCPC_MINER=false; BTCPC_STORAGE=false ;;
  3) BTCPC_CLOCK=true;  BTCPC_MINER=true;  BTCPC_STORAGE=true  ;;
  *) BTCPC_CLOCK=true;  BTCPC_MINER=true;  BTCPC_STORAGE=false ;;
esac

# ── Download binary ───────────────────────────────────────────────────────────
say "Downloading BTCPC node (${PLATFORM}/${ARCH_TAG})..."

TMP="$(mktemp)"
if curl -fsSL --progress-bar "$DOWNLOAD_URL" -o "$TMP" 2>/dev/null; then
  ok "Downloaded binary."
else
  say "Prebuilt binary not yet available for ${PLATFORM}/${ARCH_TAG}."
  say "Building from source (requires Rust — this takes ~5 minutes)..."
  if ! command -v cargo >/dev/null 2>&1; then
    say "Installing Rust..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
    export PATH="$HOME/.cargo/bin:$PATH"
  fi
  BUILD_DIR="$(mktemp -d)"
  git clone --depth=1 "${BTCPC_REPO_URL:-https://github.com/btcpc-network/btcpc}" "$BUILD_DIR" 2>/dev/null
  cd "$BUILD_DIR/rust/btcpc-node"
  cargo build --release --quiet
  cp target/release/btcpc-node "$TMP"
  cd - >/dev/null
  rm -rf "$BUILD_DIR"
  ok "Built from source."
fi

chmod +x "$TMP"

# Verify it runs
if ! "$TMP" --version >/dev/null 2>&1; then
  err "Downloaded binary failed to run. Try again or report at https://btcpc.net/help"
fi

# Install binary
if install -m 755 "$TMP" "$INSTALL_BIN" 2>/dev/null; then
  ok "Installed to $INSTALL_BIN"
elif sudo install -m 755 "$TMP" "$INSTALL_BIN" 2>/dev/null; then
  ok "Installed to $INSTALL_BIN (sudo)"
else
  mkdir -p "$HOME/.local/bin"
  install -m 755 "$TMP" "$HOME/.local/bin/btcpc-node"
  INSTALL_BIN="$HOME/.local/bin/btcpc-node"
  ok "Installed to $INSTALL_BIN (user-local)"
  export PATH="$HOME/.local/bin:$PATH"
fi
rm -f "$TMP"

# ── Data directory ────────────────────────────────────────────────────────────
mkdir -p "$DATA_DIR"

# ── Generate posting key (deterministic from account + machine-id) ────────────
say "Generating key for @${BTCPC_ACCOUNT}..."
MACHINE_ID="$(cat /etc/machine-id 2>/dev/null || hostname)"
POSTING_KEY="$(printf '%s:%s' "$BTCPC_ACCOUNT" "$MACHINE_ID" | sha256sum | awk '{print $1}')"

# ── Systemd service (Linux) ───────────────────────────────────────────────────
if [ "$PLATFORM" = "linux" ] && command -v systemctl >/dev/null 2>&1; then
  GENESIS_FILE="/usr/local/share/btcpc/genesis.json"

  # Download genesis if not present
  if [ ! -f "$GENESIS_FILE" ]; then
    sudo mkdir -p "$(dirname "$GENESIS_FILE")" 2>/dev/null || mkdir -p "$HOME/.btcpc"
    GENESIS_FILE="$HOME/.btcpc/genesis.json"
    curl -fsSL https://btcpc.net/genesis.json -o "$GENESIS_FILE" 2>/dev/null || true
  fi

  SERVICE_DIR="$HOME/.config/systemd/user"
  mkdir -p "$SERVICE_DIR"
  cat > "$SERVICE_DIR/btcpc-node.service" <<SERVICE
[Unit]
Description=BTCPC Node (@${BTCPC_ACCOUNT})
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${INSTALL_BIN}
Environment="BTCPC_ACCOUNT=${BTCPC_ACCOUNT}"
Environment="BTCPC_POSTING_KEY=${POSTING_KEY}"
Environment="BTCPC_CHAIN_ID=btcpc-1"
Environment="BTCPC_DATA_DIR=${DATA_DIR}"
Environment="BTCPC_API_PORT=4242"
Environment="BTCPC_P2P_PORT=6942"
Environment="BTCPC_CLOCK=${BTCPC_CLOCK}"
Environment="BTCPC_MINER=${BTCPC_MINER}"
Environment="BTCPC_STORAGE=${BTCPC_STORAGE}"
Environment="BTCPC_GENESIS_FILE=${GENESIS_FILE}"
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
SERVICE

  systemctl --user daemon-reload
  systemctl --user enable btcpc-node
  systemctl --user start  btcpc-node

  sleep 2
  if systemctl --user is-active --quiet btcpc-node; then
    ok "Node is running (systemd user service)."
  else
    say "Service may still be starting. Check: systemctl --user status btcpc-node"
  fi

elif [ "$PLATFORM" = "mac" ]; then
  # macOS launchd plist
  PLIST="$HOME/Library/LaunchAgents/net.btcpc.node.plist"
  cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>net.btcpc.node</string>
  <key>ProgramArguments</key>
  <array><string>${INSTALL_BIN}</string></array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>BTCPC_ACCOUNT</key><string>${BTCPC_ACCOUNT}</string>
    <key>BTCPC_POSTING_KEY</key><string>${POSTING_KEY}</string>
    <key>BTCPC_CHAIN_ID</key><string>btcpc-1</string>
    <key>BTCPC_DATA_DIR</key><string>${DATA_DIR}</string>
    <key>BTCPC_API_PORT</key><string>4242</string>
    <key>BTCPC_P2P_PORT</key><string>6942</string>
    <key>BTCPC_CLOCK</key><string>${BTCPC_CLOCK}</string>
    <key>BTCPC_MINER</key><string>${BTCPC_MINER}</string>
    <key>BTCPC_STORAGE</key><string>${BTCPC_STORAGE}</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>${DATA_DIR}/node.log</string>
  <key>StandardErrorPath</key><string>${DATA_DIR}/node.log</string>
</dict>
</plist>
PLIST
  launchctl load "$PLIST"
  ok "Node started (launchd). Logs: $DATA_DIR/node.log"
fi

# ── Ollama (miner only) ───────────────────────────────────────────────────────
if [ "$BTCPC_MINER" = "true" ] && ! command -v ollama >/dev/null 2>&1; then
  say "Installing Ollama for inference mining..."
  curl -fsSL https://ollama.com/install.sh | sh 2>/dev/null || true
fi

# ── Done ──────────────────────────────────────────────────────────────────────
echo ""
ok "BTCPC node installed and running."
echo ""
echo "  Account:    @${BTCPC_ACCOUNT}"
echo "  Roles:      clock=${BTCPC_CLOCK} miner=${BTCPC_MINER} storage=${BTCPC_STORAGE}"
echo "  API:        http://localhost:4242"
echo "  Wallet:     https://btcpc.net/app"
echo "  Logs:       journalctl --user -u btcpc-node -f"
echo ""
say "Rewards land every 30 seconds. Welcome to the network."

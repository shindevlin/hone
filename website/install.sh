#!/usr/bin/env bash
# HONE Node Installer
# Usage: curl -fsSL https://honemesh.net/install.sh | bash
#
# Self-heal rule: this installer NEVER asks a non-technical user to run a
# command on failure. Every fallible step auto-recovers — retries with
# backoff, picks a sane default, or degrades gracefully. It never `exit 1`s;
# the only hard stop is a graceful `exit 0` on a genuinely unsupported host.

ORANGE='\033[38;5;208m'
GREEN='\033[0;32m'
RED='\033[0;31m'
RESET='\033[0m'

say()  { printf "${ORANGE}[hone]${RESET} %s\n" "$1"; }
ok()   { printf "${GREEN}[ok]${RESET} %s\n" "$1"; }
warn() { printf "${RED}[hone]${RESET} %s\n" "$1" >&2; }

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
  CYGWIN*|MINGW*|MSYS*)
    warn "This looks like Windows — please use the Windows installer at https://honemesh.net/install"
    exit 0 ;;
  *)
    warn "Unsupported OS '$OS' — nothing to install here. See https://honemesh.net/help"
    exit 0 ;;
esac

case "$ARCH" in
  x86_64|amd64)   ARCH_TAG="amd64" ;;
  aarch64|arm64)  ARCH_TAG="arm64" ;;
  *) ARCH_TAG="amd64" ;;
esac

BIN_NAME="hone-node-${PLATFORM}-${ARCH_TAG}"
DOWNLOAD_URL="https://honemesh.net/downloads/${BIN_NAME}"
INSTALL_BIN="/usr/local/bin/hone-node"
DATA_DIR="${HONE_DATA_DIR:-$HOME/.hone}"

# ── Account name (non-interactive fallback to a guest name) ───────────────────
if [ -z "${HONE_ACCOUNT:-}" ]; then
  if [ -t 0 ]; then
    printf "${ORANGE}[hone]${RESET} Enter your HONE username (letters, numbers, hyphens): "
    read -r HONE_ACCOUNT
  fi
fi
if [ -z "${HONE_ACCOUNT:-}" ]; then
  # No TTY and no env var — generate a guest account so the install proceeds.
  HONE_ACCOUNT="guest-$(head -c4 /dev/urandom 2>/dev/null | od -An -tx1 | tr -d ' \n' || echo $RANDOM$RANDOM)"
  say "No username provided — using auto-generated account @${HONE_ACCOUNT}."
  say "You can rename it later from the wallet at https://honemesh.net/app"
fi

# ── Role selection (non-interactive fallback to clock+miner) ──────────────────
if [ -z "${HONE_ROLE_CHOICE:-}" ] && [ -t 0 ]; then
  echo ""
  say "Choose your node role:"
  echo "  1) Clock only      — lightweight, any machine, no GPU needed"
  echo "  2) Clock + Miner   — runs Ollama inference (default, GPU recommended)"
  echo "  3) Full node       — clock + miner + storage"
  printf "${ORANGE}[hone]${RESET} Choice [1/2/3, default=2]: "
  read -r HONE_ROLE_CHOICE
fi

case "${HONE_ROLE_CHOICE:-2}" in
  1) HONE_CLOCK=true;  HONE_MINER=false; HONE_STORAGE=false ;;
  3) HONE_CLOCK=true;  HONE_MINER=true;  HONE_STORAGE=true  ;;
  *) HONE_CLOCK=true;  HONE_MINER=true;  HONE_STORAGE=false ;;
esac

# ── Acquire the binary — retry with exponential backoff, never give up hard ───
say "Downloading HONE node (${PLATFORM}/${ARCH_TAG})..."

TMP="$(mktemp)"
DL_DELAYS="5 15 45 120 300"
got_binary=false

# First, try the prebuilt binary with a bounded backoff loop.
for delay in $DL_DELAYS; do
  if curl -fsSL --progress-bar "$DOWNLOAD_URL" -o "$TMP" 2>/dev/null && [ -s "$TMP" ]; then
    ok "Downloaded binary."
    got_binary=true
    break
  fi
  warn "Download failed — retrying in ${delay}s..."
  sleep "$delay"
done

# If the prebuilt binary never arrived, build from source (also retried).
if [ "$got_binary" != "true" ]; then
  say "Prebuilt binary unavailable — building from source (requires Rust, ~5 min)..."
  if ! command -v cargo >/dev/null 2>&1; then
    say "Installing Rust..."
    while true; do
      if curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path; then
        break
      fi
      warn "Rust install failed — retrying in 15s..."
      sleep 15
    done
    export PATH="$HOME/.cargo/bin:$PATH"
  fi

  BUILD_DIR="$(mktemp -d)"
  REPO_URL="${HONE_REPO_URL:-https://github.com/shindevlin/hone}"

  # Private-repo fallback: if a GITHUB_TOKEN is present, use it for auth.
  # If the clone fails without a token (private repo, no access), degrade
  # gracefully — tell the user where to get a prebuilt binary and exit 0
  # rather than dead-ending the whole install with exit 1.
  if [ -n "${GITHUB_TOKEN:-}" ]; then
    REPO_URL="https://${GITHUB_TOKEN}@github.com/shindevlin/hone"
  fi

  if ! git clone --depth=1 "$REPO_URL" "$BUILD_DIR" 2>/dev/null; then
    warn "Could not fetch the source (the repo may be private)."
    if [ -z "${GITHUB_TOKEN:-}" ]; then
      say "Set GITHUB_TOKEN=<your token> and re-run to build from a private repo,"
      say "or grab a prebuilt binary from https://honemesh.net/downloads"
    fi
    rm -rf "$BUILD_DIR"
    exit 0
  fi

  cd "$BUILD_DIR/rust/hone-node" || exit 0
  while true; do
    if cargo build --release --quiet; then
      break
    fi
    warn "Build failed — retrying in 30s (transient toolchain/network issue)..."
    sleep 30
  done
  cp target/release/hone-node "$TMP"
  cd - >/dev/null || true
  rm -rf "$BUILD_DIR"
  ok "Built from source."
fi

chmod +x "$TMP"

# ── Install the binary — try system path, then sudo, then user-local ──────────
if install -m 755 "$TMP" "$INSTALL_BIN" 2>/dev/null; then
  ok "Installed to $INSTALL_BIN"
elif sudo install -m 755 "$TMP" "$INSTALL_BIN" 2>/dev/null; then
  ok "Installed to $INSTALL_BIN (sudo)"
else
  mkdir -p "$HOME/.local/bin"
  install -m 755 "$TMP" "$HOME/.local/bin/hone-node"
  INSTALL_BIN="$HOME/.local/bin/hone-node"
  ok "Installed to $INSTALL_BIN (user-local)"
  export PATH="$HOME/.local/bin:$PATH"
fi
rm -f "$TMP"

# ── Data directory + key ──────────────────────────────────────────────────────
mkdir -p "$DATA_DIR"
say "Generating key for @${HONE_ACCOUNT}..."
MACHINE_ID="$(cat /etc/machine-id 2>/dev/null || hostname)"
POSTING_KEY="$(printf '%s:%s' "$HONE_ACCOUNT" "$MACHINE_ID" | sha256sum | awk '{print $1}')"

# ── Ollama (miner only) — retry, never fatal ──────────────────────────────────
if [ "$HONE_MINER" = "true" ] && ! command -v ollama >/dev/null 2>&1; then
  say "Installing Ollama for inference mining..."
  for delay in 5 15 45; do
    if curl -fsSL https://ollama.com/install.sh | sh 2>/dev/null; then
      ok "Ollama installed."
      break
    fi
    warn "Ollama install failed — retrying in ${delay}s (node still runs as clock without it)..."
    sleep "$delay"
  done
fi

# ── Service install + supervised start loop ───────────────────────────────────
# hone-setup wires up the platform service (systemd/launchd) and starts the
# node. It runs inside a `while true ... done` supervisor so a transient
# failure (network not up yet, port briefly taken) auto-retries instead of
# leaving the user at a dead prompt.
hone-setup() {
  if [ "$PLATFORM" = "linux" ] && command -v systemctl >/dev/null 2>&1; then
    GENESIS_FILE="$HOME/.hone/genesis.json"
    if [ ! -f "$GENESIS_FILE" ]; then
      curl -fsSL https://honemesh.net/genesis.json -o "$GENESIS_FILE" 2>/dev/null || true
    fi
    SERVICE_DIR="$HOME/.config/systemd/user"
    mkdir -p "$SERVICE_DIR"
    cat > "$SERVICE_DIR/hone-node.service" <<SERVICE
[Unit]
Description=HONE Node (@${HONE_ACCOUNT})
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=${INSTALL_BIN}
Environment="HONE_ACCOUNT=${HONE_ACCOUNT}"
Environment="HONE_POSTING_KEY=${POSTING_KEY}"
Environment="HONE_CHAIN_ID=hone"
Environment="HONE_DATA_DIR=${DATA_DIR}"
Environment="HONE_API_PORT=4242"
Environment="HONE_P2P_PORT=6942"
Environment="HONE_CLOCK=${HONE_CLOCK}"
Environment="HONE_MINER=${HONE_MINER}"
Environment="HONE_STORAGE=${HONE_STORAGE}"
Environment="HONE_GENESIS_FILE=${GENESIS_FILE}"
Restart=on-failure
RestartSec=5
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=default.target
SERVICE
    systemctl --user daemon-reload
    systemctl --user enable hone-node 2>/dev/null || true
    systemctl --user start  hone-node 2>/dev/null || true
    sleep 2
    systemctl --user is-active --quiet hone-node && return 0
    return 1

  elif [ "$PLATFORM" = "mac" ]; then
    PLIST="$HOME/Library/LaunchAgents/net.hone.node.plist"
    cat > "$PLIST" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>net.hone.node</string>
  <key>ProgramArguments</key>
  <array><string>${INSTALL_BIN}</string></array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>HONE_ACCOUNT</key><string>${HONE_ACCOUNT}</string>
    <key>HONE_POSTING_KEY</key><string>${POSTING_KEY}</string>
    <key>HONE_CHAIN_ID</key><string>hone</string>
    <key>HONE_DATA_DIR</key><string>${DATA_DIR}</string>
    <key>HONE_API_PORT</key><string>4242</string>
    <key>HONE_P2P_PORT</key><string>6942</string>
    <key>HONE_CLOCK</key><string>${HONE_CLOCK}</string>
    <key>HONE_MINER</key><string>${HONE_MINER}</string>
    <key>HONE_STORAGE</key><string>${HONE_STORAGE}</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>${DATA_DIR}/node.log</string>
  <key>StandardErrorPath</key><string>${DATA_DIR}/node.log</string>
</dict>
</plist>
PLIST
    launchctl unload "$PLIST" 2>/dev/null || true
    launchctl load  "$PLIST" 2>/dev/null && return 0
    return 1
  fi
  # No service manager — run the binary directly in the background.
  nohup "$INSTALL_BIN" >"$DATA_DIR/node.log" 2>&1 &
  return 0
}

setup_attempt=0
while true; do
  if hone-setup; then
    ok "Node is running."
    break
  fi
  setup_attempt=$((setup_attempt + 1))
  warn "Node start did not confirm (attempt ${setup_attempt}) — retrying in 15s..."
  sleep 15
  # After several minutes of retries, stop looping but leave the service
  # enabled so it can recover on its own (e.g. on next boot / network up).
  if [ "$setup_attempt" -ge 20 ]; then
    say "Service is installed and will keep retrying on its own. Check status later at https://honemesh.net/app"
    break
  fi
done

# ── Done ──────────────────────────────────────────────────────────────────────
echo ""
ok "HONE node installed."
echo ""
echo "  Account:    @${HONE_ACCOUNT}"
echo "  Roles:      clock=${HONE_CLOCK} miner=${HONE_MINER} storage=${HONE_STORAGE}"
echo "  API:        http://localhost:4242"
echo "  Wallet:     https://honemesh.net/app"
echo "  Logs:       journalctl --user -u hone-node -f"
echo ""
say "Rewards land every 30 seconds. Welcome to the network."

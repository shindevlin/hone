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

# ── License: UFL 3.4, Operational Scope: Decentralized ─────────────────────
# HONE does not run until its license is accepted (UFL Section 9), so ask
# before installing anything. Typing "I accept" (or HONE_ACCEPT_LICENSE=UFL-3.4-D
# for unattended installs) is the act of acceptance. Declining installs nothing.
# Keep the summary in step with hone_license::summary() (a Rust test checks it).
hone_license_summary() {
cat <<'HONE_LICENSE_SUMMARY'
HONE is licensed under the Usufruct License (UFL) 3.4, Operational Scope: Decentralized (LicenseRef-UFL-3.4-D).
Full text: https://github.com/shindevlin/hone/blob/main/LICENSE

Summary (the license text governs):
  * Use is free at any scale and nothing is paid. Redistribution is reserved,
    except that forks of HONE as network software may be distributed if they
    credit HONE and keep the license's canonical-source notice (Section 2A).
  * Disputes: every dispute arising from your use of HONE is decided only by
    online arbitration under the ICC Rules, before a single arbitrator seated
    in California, on an individual basis and not as a class or collective
    action. The arbitrator applies the UNIDROIT Principles, then California
    law. Each side bears its own costs and fees unless a claim or defense is
    frivolous or in bad faith.
  * Liability cap: each party's total liability to the other for all claims
    is limited, to the extent the law allows, to one US dollar in value,
    paid in money or in HONE at the paying party's choice.
  * Forks and bypassers are at their own risk: running HONE with this step
    bypassed, removed, or forged breaches the license and makes that copy a
    modified version, and the Licensor then owes you nothing. A modified
    version, fork, or reimplementation is used entirely at your own risk.
  * HONE is provided as is, without warranty.
  * Nothing here limits liability or waives rights that the law does not
    allow to be limited or waived, or stops either side seeking a court
    injunction against conduct Section 2 reserves or IP infringement.
HONE_LICENSE_SUMMARY
}
hone_license_accepted_in() { # DATA_DIR
  grep -q '"license": "UFL"' "$1/license-acceptance.json" 2>/dev/null &&
  grep -q '"version": "3.4"' "$1/license-acceptance.json" 2>/dev/null &&
  grep -q '"scope": "Decentralized"' "$1/license-acceptance.json" 2>/dev/null
}
hone_license_record() { # DATA_DIR — license, version, scope, UTC time, installer name; local only
  mkdir -p "$1" && cat > "$1/license-acceptance.json" <<HONE_LICENSE_RECORD
{
  "license": "UFL",
  "version": "3.4",
  "scope": "Decentralized",
  "accepted_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "binary": "hone-installer"
}
HONE_LICENSE_RECORD
}
hone_license_gate() { # DATA_DIR — returns 0 if accepted (now or before)
  if hone_license_accepted_in "$1"; then return 0; fi
  if [ -n "${HONE_ACCEPT_LICENSE:-}" ]; then
    if [ "$HONE_ACCEPT_LICENSE" = "UFL-3.4-D" ]; then
      echo "Accepted UFL 3.4, Operational Scope: Decentralized via HONE_ACCEPT_LICENSE=UFL-3.4-D."
      return 0
    fi
    echo "HONE_ACCEPT_LICENSE is set to \"$HONE_ACCEPT_LICENSE\". The only value that accepts HONE's license is UFL-3.4-D. Nothing was installed." >&2
    return 1
  fi
  echo ""
  hone_license_summary
  if ( exec </dev/tty ) 2>/dev/null; then
    printf 'Type "I accept" to accept and install, or anything else to stop: '
    HONE_LICENSE_ANSWER=""
    read -r HONE_LICENSE_ANSWER </dev/tty || true
    if [ "$HONE_LICENSE_ANSWER" = "I accept" ]; then return 0; fi
    echo "License not accepted. Nothing was installed."
    return 1
  fi
  echo "No terminal to ask on. To accept the license above and install, run again with"
  echo "HONE_ACCEPT_LICENSE=UFL-3.4-D set. Setting it is the act of acceptance. Nothing was installed."
  return 1
}
if hone_license_gate "$DATA_DIR"; then
  hone_license_record "$DATA_DIR" || warn "Could not save the license record in $DATA_DIR; the node will ask again."
else
  exit 0
fi

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
# Exit status 3 = HONE's license not accepted; do not restart-loop on it.
RestartPreventExitStatus=3
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

#!/usr/bin/env bash
# BTCPC Universal Installer
# Supports: macOS, Linux (apt/yum/pacman/apk), Windows WSL
# Usage: curl https://btcpc.net/install.sh | bash

set -euo pipefail

REPO_URL="${BTCPC_REPO_URL:-https://github.com/shindevlin/btcpc.git}"
INSTALL_DIR="${BTCPC_INSTALL_DIR:-$HOME/.btcpc}"
NODE_VERSION="${BTCPC_NODE_VERSION:-20}"
ORANGE='\033[38;5;208m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
RESET='\033[0m'

say() { printf "${ORANGE}[btcpc]${RESET} %s\n" "$1"; }
ok()  { printf "${GREEN}[ok]${RESET} %s\n" "$1"; }
err() { printf "${ORANGE}[error]${RESET} %s\n" "$1" >&2; }

cat <<'BANNER'

  ██████╗ ████████╗ ██████╗██████╗  ██████╗
  ██╔══██╗╚══██╔══╝██╔════╝██╔══██╗██╔════╝
  ██████╔╝   ██║   ██║     ██████╔╝██║
  ██╔══██╗   ██║   ██║     ██╔═══╝ ██║
  ██████╔╝   ██║   ╚██████╗██║     ╚██████╗
  ╚═════╝    ╚═╝    ╚═════╝╚═╝      ╚═════╝

  Bitcoin Proof of Compute — Universal Installer

BANNER

# ──────────────────────────────────────────────────────────────────
# Detect OS
# ──────────────────────────────────────────────────────────────────
OS="$(uname -s)"
case "$OS" in
  Darwin*)  PLATFORM="mac"; PKG="brew" ;;
  Linux*)
    PLATFORM="linux"
    if   command -v apt-get  >/dev/null 2>&1; then PKG="apt"
    elif command -v dnf      >/dev/null 2>&1; then PKG="dnf"
    elif command -v yum      >/dev/null 2>&1; then PKG="yum"
    elif command -v pacman   >/dev/null 2>&1; then PKG="pacman"
    elif command -v apk      >/dev/null 2>&1; then PKG="apk"
    else PKG="unknown"
    fi
    if grep -qi microsoft /proc/version 2>/dev/null; then
      PLATFORM="wsl"
    fi
    ;;
  CYGWIN*|MINGW*|MSYS*)
    err "Native Windows is not supported. Use the Desktop App or Docker instead:"
    err "  https://btcpc.net/install"
    exit 1
    ;;
  *) err "Unknown OS: $OS"; exit 1 ;;
esac

ARCH="$(uname -m)"
say "Detected: $PLATFORM ($ARCH) using $PKG"

# ──────────────────────────────────────────────────────────────────
# Install helper
# ──────────────────────────────────────────────────────────────────
need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    say "Installing $1..."
    case "$PKG" in
      brew)
        if ! command -v brew >/dev/null 2>&1; then
          say "Homebrew not found. Installing it first..."
          /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
        fi
        brew install "$1"
        ;;
      apt)    sudo apt-get update -qq && sudo apt-get install -y "$1" ;;
      dnf)    sudo dnf install -y "$1" ;;
      yum)    sudo yum install -y "$1" ;;
      pacman) sudo pacman -Sy --noconfirm "$1" ;;
      apk)    sudo apk add --no-cache "$1" ;;
      *) err "Don't know how to install $1 on $PLATFORM"; exit 1 ;;
    esac
  fi
}

# ──────────────────────────────────────────────────────────────────
# 1. Required tools
# ──────────────────────────────────────────────────────────────────
say "Checking required tools..."
need_cmd curl
need_cmd git

# ──────────────────────────────────────────────────────────────────
# 2. Node.js (via nvm — no sudo, no system pollution)
# ──────────────────────────────────────────────────────────────────
NODE_OK=false
if command -v node >/dev/null 2>&1; then
  CURRENT_NODE="$(node -v | sed 's/^v//' | cut -d. -f1)"
  if [ "$CURRENT_NODE" -ge "$NODE_VERSION" ]; then
    NODE_OK=true
    ok "Node.js $(node -v) already installed"
  fi
fi

if [ "$NODE_OK" = "false" ]; then
  say "Installing Node.js $NODE_VERSION via nvm..."
  if [ ! -d "$HOME/.nvm" ]; then
    curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.7/install.sh | bash
  fi
  export NVM_DIR="$HOME/.nvm"
  # shellcheck disable=SC1091
  [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"
  nvm install "$NODE_VERSION"
  nvm use "$NODE_VERSION"
  ok "Node.js $(node -v) installed"
fi

# ──────────────────────────────────────────────────────────────────
# 3. Ollama
# ──────────────────────────────────────────────────────────────────
if ! command -v ollama >/dev/null 2>&1; then
  say "Installing Ollama..."
  if [ "$PLATFORM" = "mac" ]; then
    if command -v brew >/dev/null 2>&1; then
      brew install ollama
    else
      err "Please install Ollama manually: https://ollama.com/download/mac"
      err "Then re-run this installer."
      exit 1
    fi
  else
    curl -fsSL https://ollama.com/install.sh | sh
  fi
  ok "Ollama installed"
else
  ok "Ollama already installed"
fi

if ! pgrep -f "ollama serve" >/dev/null 2>&1; then
  say "Starting Ollama..."
  nohup ollama serve >/dev/null 2>&1 &
  sleep 2
fi

# ──────────────────────────────────────────────────────────────────
# 4. Clone BTCPC
# ──────────────────────────────────────────────────────────────────
if [ -d "$INSTALL_DIR/.git" ]; then
  say "BTCPC already installed at $INSTALL_DIR — updating..."
  cd "$INSTALL_DIR"
  git pull --ff-only || true
else
  say "Cloning BTCPC to $INSTALL_DIR..."
  git clone "$REPO_URL" "$INSTALL_DIR"
  cd "$INSTALL_DIR"
fi

# ──────────────────────────────────────────────────────────────────
# 5. npm install
# ──────────────────────────────────────────────────────────────────
say "Installing dependencies (this takes a minute)..."
npm install --silent

# ──────────────────────────────────────────────────────────────────
# 6. MongoDB (Docker if available)
# ──────────────────────────────────────────────────────────────────
MONGO_RUNNING=false
if nc -z localhost 27017 2>/dev/null; then
  MONGO_RUNNING=true
  ok "MongoDB already running on localhost:27017"
fi

if [ "$MONGO_RUNNING" = "false" ]; then
  if command -v docker >/dev/null 2>&1; then
    say "Starting MongoDB via Docker..."
    docker run -d --name btcpc-mongo \
      -p 27017:27017 \
      -e MONGO_INITDB_ROOT_USERNAME=root \
      -e MONGO_INITDB_ROOT_PASSWORD=example \
      --restart unless-stopped \
      mongo:7 >/dev/null 2>&1 || docker start btcpc-mongo >/dev/null
    sleep 3
    ok "MongoDB running in Docker container 'btcpc-mongo'"
  else
    err "MongoDB not found and Docker not available."
    err "Install Docker first (https://docs.docker.com/get-docker/), then re-run this script."
    exit 1
  fi
fi

# ──────────────────────────────────────────────────────────────────
# 7. Configure .env
# ──────────────────────────────────────────────────────────────────
if [ ! -f "$INSTALL_DIR/.env" ]; then
  say "Creating .env..."
  cat > "$INSTALL_DIR/.env" <<EOF
NODE_ENV=production
PORT=3000
MONGODB_URI=mongodb://root:example@localhost:27017/btcpc?authSource=admin
OLLAMA_URL=http://localhost:11434
JWT_SECRET=$(openssl rand -hex 32)
JWT_EXPIRES_IN=7d
EOF
  ok ".env created"
fi

# ──────────────────────────────────────────────────────────────────
# 8. Launch the setup wizard
# ──────────────────────────────────────────────────────────────────
echo
ok "Installation complete!"
echo
say "Launching the setup wizard..."
say "(this will ask whether to mine, run a clock, and create your account)"
echo
sleep 1
exec node "$INSTALL_DIR/bin/btcpc-setup"

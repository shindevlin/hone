#!/usr/bin/env bash
# BTCPC Universal Installer - Self-Healing Edition
# Supports: macOS, Linux (apt/yum/pacman/apk), Windows WSL
# Usage: curl https://btcpc.net/install.sh | bash
# Database is optional (Phase F) - no database install required.

REPO_URL="${BTCPC_REPO_URL:-https://github.com/shindevlin/btcpc.git}"
INSTALL_DIR="${BTCPC_INSTALL_DIR:-$HOME/.btcpc}"
NODE_VERSION="${BTCPC_NODE_VERSION:-20}"
ORANGE='\033[38;5;208m'
GREEN='\033[0;32m'
RESET='\033[0m'

say() { printf "${ORANGE}[btcpc]${RESET} %s\n" "$1"; }
ok()  { printf "${GREEN}[ok]${RESET} %s\n" "$1"; }

cat <<'BANNER'

  ######   ######## ######  ######  ######
  ##  ##      ##    ##      ##  ##  ##
  ######      ##    ##      ######  ##
  ##  ##      ##    ##      ##      ##
  ######      ##    ######  ##      ######

  Bitcoin Proof of Compute -- Universal Installer

BANNER

# ------------------------------------------------------------------
# Detect OS
# ------------------------------------------------------------------
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
    say "Native Windows shell detected. Use the .bat or .ps1 installer instead:"
    say "  https://btcpc.net/install"
    say "Sleeping 30 seconds before exiting."
    sleep 30
    exit 0
    ;;
  *)
    say "Unknown OS: $OS. Attempting to continue anyway..."
    PLATFORM="unknown"; PKG="unknown"
    ;;
esac

ARCH="$(uname -m)"
say "Detected: $PLATFORM ($ARCH) using $PKG"

# ------------------------------------------------------------------
# Install helper (with retry)
# ------------------------------------------------------------------
need_cmd() {
  local cmd="$1"
  if command -v "$cmd" >/dev/null 2>&1; then
    return 0
  fi
  say "Installing $cmd..."
  local installed=false
  while [ "$installed" = "false" ]; do
    case "$PKG" in
      brew)
        if ! command -v brew >/dev/null 2>&1; then
          say "Homebrew not found. Installing it first..."
          /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
        fi
        brew install "$cmd" && installed=true || true
        ;;
      apt)    sudo apt-get update -qq && sudo apt-get install -y "$cmd" && installed=true || true ;;
      dnf)    sudo dnf install -y "$cmd" && installed=true || true ;;
      yum)    sudo yum install -y "$cmd" && installed=true || true ;;
      pacman) sudo pacman -Sy --noconfirm "$cmd" && installed=true || true ;;
      apk)    sudo apk add --no-cache "$cmd" && installed=true || true ;;
      *)
        say "Cannot auto-install $cmd on $PLATFORM. Retrying in 30 seconds..."
        sleep 30
        ;;
    esac
    if [ "$installed" = "false" ]; then
      say "Installation of $cmd failed. Retrying in 15 seconds..."
      sleep 15
    fi
  done
  ok "$cmd installed"
}

# ------------------------------------------------------------------
# 1. Required tools: curl, git
# ------------------------------------------------------------------
say "Checking required tools..."
need_cmd curl
need_cmd git

# ------------------------------------------------------------------
# 2. Node.js (via nvm - no sudo, no system pollution)
# ------------------------------------------------------------------
NODE_OK=false
if command -v node >/dev/null 2>&1; then
  CURRENT_NODE="$(node -v | sed 's/^v//' | cut -d. -f1)"
  if [ "$CURRENT_NODE" -ge "$NODE_VERSION" ] 2>/dev/null; then
    NODE_OK=true
    ok "Node.js $(node -v) already installed"
  fi
fi

if [ "$NODE_OK" = "false" ]; then
  say "Installing Node.js $NODE_VERSION via nvm..."
  while true; do
    if [ ! -d "$HOME/.nvm" ]; then
      curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.7/install.sh | bash && break || true
    else
      break
    fi
    say "nvm install failed. Retrying in 15 seconds..."
    sleep 15
  done
  export NVM_DIR="$HOME/.nvm"
  # shellcheck disable=SC1091
  [ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh"
  while ! nvm install "$NODE_VERSION" 2>/dev/null; do
    say "nvm install $NODE_VERSION failed. Retrying in 15 seconds..."
    sleep 15
  done
  nvm use "$NODE_VERSION" 2>/dev/null || true
  ok "Node.js $(node -v) installed"
fi

# ------------------------------------------------------------------
# 3. Ollama (always auto-install silently)
# ------------------------------------------------------------------
if ! command -v ollama >/dev/null 2>&1; then
  say "Installing Ollama..."
  while true; do
    if curl -fsSL https://ollama.com/install.sh | sh; then
      break
    fi
    say "Ollama install failed. Retrying in 15 seconds..."
    sleep 15
  done
  ok "Ollama installed"
else
  ok "Ollama already installed"
fi

if ! pgrep -f "ollama serve" >/dev/null 2>&1; then
  say "Starting Ollama..."
  nohup ollama serve >/dev/null 2>&1 &
  sleep 2
fi

# ------------------------------------------------------------------
# 3b. CUDA / GPU detection (WSL + Linux with NVIDIA)
# ------------------------------------------------------------------
if command -v nvidia-smi >/dev/null 2>&1; then
  ok "NVIDIA GPU detected: $(nvidia-smi --query-gpu=name --format=csv,noheader 2>/dev/null | head -1)"
elif [ -f /usr/lib/wsl/lib/libcuda.so ] || [ -f /usr/lib/wsl/lib/libcuda.so.1 ]; then
  say "WSL NVIDIA driver detected but nvidia-smi not found"
  say "Installing CUDA toolkit for WSL..."
  case "$PKG" in
    apt)
      # WSL already has the driver from Windows — just need the toolkit
      if ! dpkg -l cuda-toolkit-12-* >/dev/null 2>&1; then
        wget -q https://developer.download.nvidia.com/compute/cuda/repos/wsl-ubuntu/x86_64/cuda-keyring_1.1-1_all.deb -O /tmp/cuda-keyring.deb 2>/dev/null && \
        sudo dpkg -i /tmp/cuda-keyring.deb 2>/dev/null && \
        sudo apt-get update -qq 2>/dev/null && \
        sudo apt-get install -y cuda-toolkit-12-6 2>/dev/null && \
        ok "CUDA toolkit installed" || say "CUDA install failed (non-fatal — Ollama may still use GPU)"
      else
        ok "CUDA toolkit already installed"
      fi
      ;;
    *) say "CUDA auto-install only supported on apt-based systems. Install manually if needed." ;;
  esac
elif lspci 2>/dev/null | grep -qi nvidia; then
  say "NVIDIA GPU detected but no driver installed"
  say "Installing NVIDIA driver + CUDA..."
  case "$PKG" in
    apt)
      sudo apt-get update -qq 2>/dev/null && \
      sudo apt-get install -y nvidia-driver-535 nvidia-cuda-toolkit 2>/dev/null && \
      ok "NVIDIA driver + CUDA installed (reboot may be required)" || \
      say "NVIDIA driver install failed (non-fatal — Ollama will use CPU)"
      ;;
    dnf|yum)
      sudo $PKG install -y nvidia-driver cuda-toolkit 2>/dev/null && \
      ok "NVIDIA driver + CUDA installed" || say "NVIDIA install failed (non-fatal)"
      ;;
    pacman)
      sudo pacman -Sy --noconfirm nvidia nvidia-utils cuda 2>/dev/null && \
      ok "NVIDIA + CUDA installed" || say "NVIDIA install failed (non-fatal)"
      ;;
    *) say "Install NVIDIA drivers manually for GPU mining. Ollama will use CPU for now." ;;
  esac
else
  say "No NVIDIA GPU detected — mining will use CPU (slower but still works)"
fi

# ------------------------------------------------------------------
# 4. Clone BTCPC
# ------------------------------------------------------------------
if [ -d "$INSTALL_DIR/.git" ]; then
  say "BTCPC already installed at $INSTALL_DIR -- updating..."
  cd "$INSTALL_DIR"
  git pull --ff-only 2>/dev/null || true
else
  say "Cloning BTCPC to $INSTALL_DIR..."
  if ! git clone "$REPO_URL" "$INSTALL_DIR" 2>/dev/null; then
    # Try with GITHUB_TOKEN if set
    if [ -n "${GITHUB_TOKEN:-}" ]; then
      say "Retrying clone with GITHUB_TOKEN..."
      AUTHED_URL="$(echo "$REPO_URL" | sed "s|https://|https://${GITHUB_TOKEN}@|")"
      if ! git clone "$AUTHED_URL" "$INSTALL_DIR" 2>/dev/null; then
        say "Clone failed even with GITHUB_TOKEN. This repo may be private and your token may not have access."
        say "Set GITHUB_TOKEN to a valid personal access token with repo scope and retry."
        say "Sleeping 30 seconds before exiting."
        sleep 30
        exit 0
      fi
    else
      say "Clone failed. This repo may be private. Set GITHUB_TOKEN env var with repo access and retry."
      say "Sleeping 30 seconds before exiting."
      sleep 30
      exit 0
    fi
  fi
  cd "$INSTALL_DIR"
fi

# ------------------------------------------------------------------
# 5. npm install
# ------------------------------------------------------------------
say "Installing dependencies (this takes a minute)..."
while ! npm install --silent 2>/dev/null; do
  say "npm install failed. Retrying in 15 seconds..."
  sleep 15
done
ok "dependencies installed"

# ------------------------------------------------------------------
# 6. Configure .env (database is optional - not required)
# ------------------------------------------------------------------
if [ ! -f "$INSTALL_DIR/.env" ]; then
  say "Creating .env..."
  JWT_SECRET="$(openssl rand -hex 32 2>/dev/null || cat /dev/urandom | head -c 32 | xxd -p | tr -d '\n')"
  cat > "$INSTALL_DIR/.env" <<EOF
NODE_ENV=production
PORT=3000
OLLAMA_URL=http://localhost:11434
JWT_SECRET=${JWT_SECRET}
JWT_EXPIRES_IN=7d
EOF
  ok ".env created"
fi

# ------------------------------------------------------------------
# 7. Launch the setup wizard (restart automatically on crash)
# ------------------------------------------------------------------
echo
ok "Installation complete!"
echo
say "Launching the BTCPC intelligent installer..."
say "(will detect your AI engines and ask what you want to run)"
echo
sleep 1
cd "$INSTALL_DIR"
node bin/btcpc-install

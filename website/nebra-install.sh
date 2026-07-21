#!/bin/bash
# Install HONE on a Nebra Helium Hotspot
# Usage: curl -fsSL https://honemesh.net/nebra-install.sh | bash
#
# Shin Devlin — HONE Project
# https://honemesh.net
#
# This script:
#   1. Detects ARM architecture + Nebra-specific files
#   2. Installs Node.js via nvm (ARM build)
#   3. Clones the hone repo
#   4. npm install
#   5. Prompts for HONE account name (or defaults to gateway-<random>)
#   6. Auto-detects LoRa region from Nebra's existing config
#   7. Registers as a HONE gateway
#   8. Installs systemd service: hone-nebra.service
#   9. Starts and enables

set -e

HONE_REPO="https://github.com/hone-network/hone.git"
HONE_BRANCH="main"
HONE_DIR="$HOME/hone"
SERVICE_NAME="hone-nebra"
NVM_VERSION="v0.39.7"
NODE_VERSION="20"

# ─── Colors ──────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

info()    { echo -e "${BLUE}[hone]${NC} $1"; }
success() { echo -e "${GREEN}[hone]${NC} $1"; }
warn()    { echo -e "${YELLOW}[hone]${NC} $1"; }
error()   { echo -e "${RED}[hone]${NC} $1" >&2; }

# ─── Architecture check ──────────────────────────────────────────
ARCH=$(uname -m)
info "Architecture: $ARCH"

if [[ "$ARCH" != "armv7l" && "$ARCH" != "aarch64" && "$ARCH" != "arm64" ]]; then
  warn "Not running on ARM. This installer is designed for Nebra Helium Hotspots."
  warn "Continuing anyway for x86 development machines."
fi

# ─── Nebra detection ─────────────────────────────────────────────
IS_NEBRA=0
if [[ -f /etc/nebra/config.json ]] || [[ -d /opt/nebra ]] || [[ -f /usr/bin/balena-engine ]]; then
  IS_NEBRA=1
  info "Nebra hardware detected."
else
  warn "Nebra config not found. Running in generic SBC mode."
fi

# ─── LoRa region auto-detect ─────────────────────────────────────
LORA_REGION=""
if [[ -f /etc/nebra/config.json ]]; then
  LORA_REGION=$(python3 -c "import json; d=json.load(open('/etc/nebra/config.json')); print(d.get('region',''))" 2>/dev/null || true)
fi
if [[ -z "$LORA_REGION" ]] && [[ -f /etc/lora_pkt_fwd/global_conf.json ]]; then
  # Infer region from frequency in packet forwarder config
  FREQ=$(python3 -c "
import json
try:
  d=json.load(open('/etc/lora_pkt_fwd/global_conf.json'))
  f=d.get('SX130x_conf',{}).get('chan_multiSF_0',{}).get('freq',0)
  print(f)
except:
  print(0)
" 2>/dev/null || echo 0)
  if [[ "$FREQ" -ge 902000000 ]] && [[ "$FREQ" -lt 928000000 ]]; then LORA_REGION="US915"; fi
  if [[ "$FREQ" -ge 863000000 ]] && [[ "$FREQ" -lt 870000000 ]]; then LORA_REGION="EU868"; fi
  if [[ "$FREQ" -ge 915000000 ]] && [[ "$FREQ" -lt 928000000 ]]; then LORA_REGION="AU915"; fi
  if [[ "$FREQ" -ge 923000000 ]] && [[ "$FREQ" -lt 925000000 ]]; then LORA_REGION="AS923"; fi
  if [[ "$FREQ" -ge 865000000 ]] && [[ "$FREQ" -lt 867000000 ]]; then LORA_REGION="IN865"; fi
fi
if [[ -z "$LORA_REGION" ]]; then
  LORA_REGION="US915"
  warn "Could not detect LoRa region. Defaulting to US915."
fi
info "LoRa region: $LORA_REGION"

# ─── Account name prompt ─────────────────────────────────────────
RANDOM_SUFFIX=$(cat /dev/urandom | tr -dc 'a-z0-9' | fold -w 4 | head -n 1 2>/dev/null || echo "0001")
DEFAULT_ACCOUNT="gateway-${RANDOM_SUFFIX}"
if [[ -t 0 ]]; then
  read -p "$(echo -e "${YELLOW}Enter your HONE account name [${DEFAULT_ACCOUNT}]:${NC} ")" HONE_ACCOUNT
  HONE_ACCOUNT="${HONE_ACCOUNT:-$DEFAULT_ACCOUNT}"
else
  HONE_ACCOUNT="$DEFAULT_ACCOUNT"
  info "Non-interactive mode. Using account: $HONE_ACCOUNT"
fi
info "HONE account: $HONE_ACCOUNT"

GATEWAY_NAME="${HONE_ACCOUNT}-nebra"

# ─── GPS coordinates (optional) ──────────────────────────────────
HONE_LAT="0"
HONE_LON="0"
if [[ -t 0 ]]; then
  read -p "$(echo -e "${YELLOW}Enter GPS latitude (optional, press Enter to skip):${NC} ")" HONE_LAT_INPUT
  read -p "$(echo -e "${YELLOW}Enter GPS longitude (optional, press Enter to skip):${NC} ")" HONE_LON_INPUT
  HONE_LAT="${HONE_LAT_INPUT:-0}"
  HONE_LON="${HONE_LON_INPUT:-0}"
fi

# ─── Node.js via nvm ─────────────────────────────────────────────
if ! command -v node &>/dev/null || [[ "$(node --version | cut -d. -f1 | tr -d v)" -lt 18 ]]; then
  info "Installing Node.js $NODE_VERSION via nvm..."
  export NVM_DIR="$HOME/.nvm"
  if [[ ! -d "$NVM_DIR" ]]; then
    curl -o- "https://raw.githubusercontent.com/nvm-sh/nvm/${NVM_VERSION}/install.sh" | bash
  fi
  # shellcheck source=/dev/null
  source "$NVM_DIR/nvm.sh"
  nvm install "$NODE_VERSION"
  nvm use "$NODE_VERSION"
  nvm alias default "$NODE_VERSION"
  success "Node.js $(node --version) installed."
else
  info "Node.js $(node --version) already installed."
fi

# Ensure nvm loads in shells
if ! grep -q 'nvm.sh' "$HOME/.bashrc" 2>/dev/null; then
  cat >> "$HOME/.bashrc" << 'NVMEOF'

# nvm (added by hone-nebra installer)
export NVM_DIR="$HOME/.nvm"
[ -s "$NVM_DIR/nvm.sh" ] && source "$NVM_DIR/nvm.sh"
[ -s "$NVM_DIR/bash_completion" ] && source "$NVM_DIR/bash_completion"
NVMEOF
fi

# ─── Clone/update repo ──────────────────────────────────────────
if [[ -d "$HONE_DIR/.git" ]]; then
  info "Updating existing hone repo at $HONE_DIR..."
  git -C "$HONE_DIR" fetch origin "$HONE_BRANCH" --quiet
  git -C "$HONE_DIR" reset --hard "origin/$HONE_BRANCH" --quiet
else
  info "Cloning hone repo to $HONE_DIR..."
  git clone --branch "$HONE_BRANCH" --depth 1 "$HONE_REPO" "$HONE_DIR"
fi
success "Repo ready at $HONE_DIR"

# ─── npm install ────────────────────────────────────────────────
info "Installing npm dependencies..."
cd "$HONE_DIR"
npm install --omit=dev --quiet
success "Dependencies installed."

# ─── .env configuration ─────────────────────────────────────────
ENV_FILE="$HONE_DIR/.env"
info "Writing $ENV_FILE..."
cat > "$ENV_FILE" << ENVEOF
HONE_MINER=$HONE_ACCOUNT
HONE_GATEWAY_NAME=$GATEWAY_NAME
HONE_LORA_REGION=$LORA_REGION
HONE_LAT=$HONE_LAT
HONE_LON=$HONE_LON
HONE_NODE_URL=https://honemesh.net/testnet
HONE_ROLES=gateway,clock
ENVEOF
success ".env written."

# ─── systemd service ────────────────────────────────────────────
NODE_BIN=$(command -v node || echo "$HOME/.nvm/versions/node/$(node --version 2>/dev/null)/bin/node")
SERVICE_FILE="$HOME/.config/systemd/user/${SERVICE_NAME}.service"
mkdir -p "$(dirname "$SERVICE_FILE")"

info "Writing systemd service: $SERVICE_FILE"
cat > "$SERVICE_FILE" << SVCEOF
[Unit]
Description=HONE Nebra LoRa Gateway Daemon
After=network.target
Wants=network-online.target

[Service]
Type=simple
WorkingDirectory=${HONE_DIR}
EnvironmentFile=${ENV_FILE}
ExecStart=${NODE_BIN} ${HONE_DIR}/bin/hone-nebra --account ${HONE_ACCOUNT} --gateway ${GATEWAY_NAME} --region ${LORA_REGION} --lat ${HONE_LAT} --lon ${HONE_LON}
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal
SyslogIdentifier=hone-nebra

[Install]
WantedBy=default.target
SVCEOF

# Reload systemd and start
systemctl --user daemon-reload
systemctl --user enable "${SERVICE_NAME}" 2>/dev/null || true
systemctl --user start "${SERVICE_NAME}" 2>/dev/null || true

# Show status
sleep 2
systemctl --user status "${SERVICE_NAME}" --no-pager || true

success ""
success "HONE Nebra gateway installed and running!"
success ""
success "Account:  $HONE_ACCOUNT"
success "Gateway:  $GATEWAY_NAME"
success "Region:   $LORA_REGION"
success ""
info "Useful commands:"
info "  systemctl --user status $SERVICE_NAME"
info "  journalctl --user -u $SERVICE_NAME -f"
info "  systemctl --user restart $SERVICE_NAME"
info ""
info "View the network map: https://honemesh.net/map.html"

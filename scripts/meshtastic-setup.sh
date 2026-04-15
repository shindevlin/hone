#!/bin/bash
# BTCPC Meshtastic Setup — earn BTCPC by relaying sensor data
# Shin Devlin
#
# Usage: curl -fsSL https://btcpc.net/meshtastic-setup.sh | bash
#
# This script:
#   1. Installs the Meshtastic Python CLI
#   2. Detects your connected Meshtastic device
#   3. Adds the "btcpc" channel to the device
#   4. Shows you how to run the BTCPC bridge
#
# Works with: T-Beam, Heltec V3, RAK WisBlock, LilyGo T-Echo,
#             Station G2, any Meshtastic-compatible device

set -e

ORANGE='\033[0;33m'
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m'

echo ""
echo -e "${ORANGE}  ____  _____ ____ ____   ____${NC}"
echo -e "${ORANGE} | __ )|_   _/ ___|  _ \\ / ___|${NC}"
echo -e "${ORANGE} |  _ \\  | || |   | |_) | |${NC}"
echo -e "${ORANGE} | |_) | | || |___|  __/| |___${NC}"
echo -e "${ORANGE} |____/  |_| \\____|_|    \\____|${NC}"
echo ""
echo -e "${ORANGE}Meshtastic Bridge Setup${NC}"
echo "Earn BTCPC by relaying sensor data over mesh radio"
echo ""

# ─── Check for Python ────────────────────────────────────────────

if ! command -v python3 &>/dev/null && ! command -v python &>/dev/null; then
  echo -e "${RED}Python not found. Install Python 3 first.${NC}"
  exit 1
fi

PYTHON=$(command -v python3 || command -v python)
PIP=$(command -v pip3 || command -v pip)

# ─── Install Meshtastic CLI ─────────────────────────────────────

echo -e "${GREEN}[1/4]${NC} Installing Meshtastic Python CLI..."
if command -v meshtastic &>/dev/null; then
  echo "  meshtastic CLI already installed: $(meshtastic --version 2>/dev/null || echo 'unknown version')"
else
  if [ -n "$PIP" ]; then
    $PIP install meshtastic 2>/dev/null || $PIP install --user meshtastic
    echo "  meshtastic CLI installed"
  else
    echo -e "${RED}  pip not found — install with: sudo apt install python3-pip${NC}"
    exit 1
  fi
fi

# ─── Detect device ───────────────────────────────────────────────

echo ""
echo -e "${GREEN}[2/4]${NC} Detecting Meshtastic device..."

DEVICE_PORT=""
for dev in /dev/ttyUSB0 /dev/ttyUSB1 /dev/ttyACM0 /dev/ttyACM1; do
  if [ -e "$dev" ]; then
    DEVICE_PORT="$dev"
    break
  fi
done

if [ -z "$DEVICE_PORT" ]; then
  # Try by-id for more specific detection
  if [ -d /dev/serial/by-id ]; then
    for f in /dev/serial/by-id/*; do
      if echo "$f" | grep -iqE "cp210|ch340|esp32|meshtastic|t-beam|heltec"; then
        DEVICE_PORT=$(readlink -f "$f")
        break
      fi
    done
  fi
fi

if [ -n "$DEVICE_PORT" ]; then
  echo "  Device found: $DEVICE_PORT"
else
  echo "  No device detected — using auto mode"
  echo "  (Connect your Meshtastic device via USB and try again)"
  DEVICE_PORT="auto"
fi

# ─── Add BTCPC channel ──────────────────────────────────────────

echo ""
echo -e "${GREEN}[3/4]${NC} Adding BTCPC channel to device..."

if meshtastic --port "$DEVICE_PORT" --ch-add btcpc 2>/dev/null; then
  echo "  BTCPC channel added"
else
  # Channel may already exist
  echo "  BTCPC channel already exists (or device not connected)"
fi

# Set channel name explicitly on the new index
meshtastic --port "$DEVICE_PORT" --ch-set name btcpc --ch-index 1 2>/dev/null || true

# ─── Done ────────────────────────────────────────────────────────

echo ""
echo -e "${GREEN}[4/4]${NC} Setup complete!"
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo -e "  Your Meshtastic device is now on the ${ORANGE}btcpc${NC} channel."
echo "  It will relay BTCPC sensor data and earn from the IoT pool."
echo ""
echo "  To run the bridge daemon:"
echo ""
echo -e "    ${GREEN}node bin/btcpc-meshtastic --account <your-btcpc-name>${NC}"
echo ""
echo "  Or install BTCPC first:"
echo ""
echo -e "    ${GREEN}curl -fsSL https://btcpc.net/install.sh | bash${NC}"
echo ""
echo "  Compatible devices:"
echo "    - LILYGO T-Beam (ESP32 + LoRa + GPS)"
echo "    - Heltec V3 (ESP32-S3 + LoRa + OLED)"
echo "    - RAK WisBlock (nRF52840 + LoRa)"
echo "    - LilyGo T-Echo (nRF52840 + LoRa + GPS + E-Ink)"
echo "    - Station G2 (ESP32-S3 + LoRa + Ethernet)"
echo "    - Any Meshtastic-compatible device"
echo ""
echo -e "  Learn more: ${ORANGE}https://btcpc.net/meshtastic${NC}"
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""

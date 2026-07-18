#!/bin/bash
# ================================================================
# HONE Nebra SD Card Preparation Script
# Shin Devlin
#
# Downloads Raspberry Pi OS Lite, customizes it with HONE pre-
# installed, and writes the image to an SD card. After flashing,
# insert the card into the Nebra, plug in power + ethernet, and
# the device boots directly into a HONE LoRa gateway node.
#
# Usage:
#   sudo bash scripts/prepare-nebra-sd.sh /dev/sdX
#
# Where /dev/sdX is the SD card device (NOT a partition like /dev/sdX1).
# Use 'lsblk' to identify the correct device. BE CAREFUL — this
# overwrites the entire card.
#
# Requirements:
#   - Linux host with internet access
#   - SD card reader
#   - 8GB+ micro SD card
#   - Root/sudo access
# ================================================================
set -euo pipefail

ORANGE='\033[38;5;208m'
GREEN='\033[0;32m'
RED='\033[0;31m'
RESET='\033[0m'

say() { printf "${ORANGE}[hone-nebra]${RESET} %s\n" "$1"; }
ok()  { printf "${GREEN}[ok]${RESET} %s\n" "$1"; }
err() { printf "${RED}[error]${RESET} %s\n" "$1" >&2; }

# ─── Validate arguments ──────────────────────────────────────────
DEVICE="${1:-}"
if [ -z "$DEVICE" ]; then
  echo "Usage: sudo bash $0 /dev/sdX"
  echo ""
  echo "Available block devices:"
  lsblk -o NAME,SIZE,TYPE,MOUNTPOINT | grep -E "disk|part"
  echo ""
  echo "Choose the SD card device (the whole disk, not a partition)."
  exit 1
fi

if [ ! -b "$DEVICE" ]; then
  err "$DEVICE is not a block device"
  exit 1
fi

# Safety check — don't flash the system disk
ROOT_DISK=$(findmnt -n -o SOURCE / | sed 's/[0-9]*$//')
if [ "$DEVICE" = "$ROOT_DISK" ]; then
  err "REFUSING to write to $DEVICE — that's your system disk!"
  exit 1
fi

DEVICE_SIZE=$(lsblk -b -d -n -o SIZE "$DEVICE" 2>/dev/null || echo 0)
if [ "$DEVICE_SIZE" -gt 137438953472 ]; then
  err "$DEVICE is $(( DEVICE_SIZE / 1073741824 )) GB — that's too large for an SD card."
  err "Are you sure this is the right device? Use lsblk to check."
  exit 1
fi

say "Target device: $DEVICE ($(( DEVICE_SIZE / 1073741824 )) GB)"
echo ""
echo "⚠️  WARNING: This will ERASE EVERYTHING on $DEVICE"
echo "    Press Ctrl+C to cancel, or Enter to continue..."
read -r

# ─── Download Raspberry Pi OS Lite ────────────────────────────────
IMG_URL="https://downloads.raspberrypi.com/raspios_lite_arm64/images/raspios_lite_arm64-2024-11-19/2024-11-19-raspios-bookworm-arm64-lite.img.xz"
IMG_DIR="/tmp/hone-nebra-image"
IMG_XZ="$IMG_DIR/raspios.img.xz"
IMG_RAW="$IMG_DIR/raspios.img"

mkdir -p "$IMG_DIR"

if [ -f "$IMG_RAW" ]; then
  ok "Raspberry Pi OS image already downloaded"
else
  say "Downloading Raspberry Pi OS Lite (64-bit)..."
  curl -fSL -o "$IMG_XZ" "$IMG_URL"
  say "Decompressing..."
  xz -d "$IMG_XZ"
  ok "Image ready: $IMG_RAW"
fi

# ─── Write image to SD card ──────────────────────────────────────
say "Writing image to $DEVICE..."
dd if="$IMG_RAW" of="$DEVICE" bs=4M status=progress conv=fsync
sync
ok "Image written to $DEVICE"

# ─── Mount the boot partition for customization ──────────────────
say "Customizing the image..."
BOOT_PART="${DEVICE}1"
ROOT_PART="${DEVICE}2"
BOOT_MNT="$IMG_DIR/boot"
ROOT_MNT="$IMG_DIR/root"

mkdir -p "$BOOT_MNT" "$ROOT_MNT"
mount "$BOOT_PART" "$BOOT_MNT"
mount "$ROOT_PART" "$ROOT_MNT"

# ─── Enable SSH ──────────────────────────────────────────────────
touch "$BOOT_MNT/ssh"
ok "SSH enabled"

# ─── Configure user (pi) with password ───────────────────────────
# Raspberry Pi OS Bookworm requires a userconf.txt file for first-boot user
# Format: username:encrypted-password
NEBRA_PASS="${HONE_NEBRA_PASS:?Set HONE_NEBRA_PASS env var}"
ENCRYPTED_PASS=$(openssl passwd -6 "$NEBRA_PASS")
echo "pi:$ENCRYPTED_PASS" > "$BOOT_MNT/userconf.txt"
ok "User 'pi' configured (password: $NEBRA_PASS)"

# ─── Write first-boot script ─────────────────────────────────────
# This script runs once on first boot, installs HONE, and sets up
# the LoRa gateway as a systemd service.
cat > "$ROOT_MNT/home/pi/hone-first-boot.sh" << 'FIRSTBOOT'
#!/bin/bash
# HONE Nebra First Boot Script
# Runs once, installs everything, starts the gateway
set -euo pipefail

LOG="/var/log/hone-first-boot.log"
exec > >(tee -a "$LOG") 2>&1

echo "[hone] First boot — $(date)"

# Wait for network
echo "[hone] Waiting for network..."
for i in $(seq 1 60); do
  if ping -c 1 -W 2 1.1.1.1 >/dev/null 2>&1; then break; fi
  sleep 2
done

# Install Node.js via nvm
echo "[hone] Installing Node.js..."
export NVM_DIR="/home/pi/.nvm"
curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.7/install.sh | bash
. "$NVM_DIR/nvm.sh"
nvm install 20
nvm use 20

# Clone HONE
echo "[hone] Cloning HONE..."
if [ -d "/home/pi/.hone" ]; then
  cd /home/pi/.hone && git pull
else
  git clone https://github.com/shindevlin/hone.git /home/pi/.hone || true
fi
cd /home/pi/.hone
npm install --omit=dev

# Create .env
if [ ! -f "/home/pi/.hone/.env" ]; then
  cat > /home/pi/.hone/.env << ENV
NODE_ENV=production
PORT=3000
HONE_MINER=${HONE_NEBRA_ACCOUNT:-shindevlin}
HONE_ROLES=clock,storage,sensor,gateway
HONE_SEED_PEERS=wss://hone-relay.shindevlin.workers.dev/ws
HONE_RELAY_URL=wss://hone-relay.shindevlin.workers.dev/ws
JWT_SECRET=$(openssl rand -hex 32)
HONE_STORAGE_CAPACITY_GB=4
P2P_PORT=6942
ENV
fi

# Install systemd service
sudo cat > /etc/systemd/system/hone-nebra.service << UNIT
[Unit]
Description=HONE Nebra Gateway
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/.hone
ExecStart=/home/pi/.nvm/versions/node/$(node -v)/bin/node bin/hone-all
Restart=always
RestartSec=10
Environment=PATH=/home/pi/.nvm/versions/node/$(node -v)/bin:/usr/local/bin:/usr/bin:/bin

[Install]
WantedBy=multi-user.target
UNIT

sudo systemctl daemon-reload
sudo systemctl enable hone-nebra
sudo systemctl start hone-nebra

echo "[hone] First boot complete — HONE gateway running"

# Remove this script from rc.local so it doesn't run again
sudo sed -i '/hone-first-boot/d' /etc/rc.local

FIRSTBOOT

chmod +x "$ROOT_MNT/home/pi/hone-first-boot.sh"

# Add to rc.local for first-boot execution
if [ -f "$ROOT_MNT/etc/rc.local" ]; then
  sed -i '/^exit 0/i bash /home/pi/hone-first-boot.sh &' "$ROOT_MNT/etc/rc.local"
else
  echo '#!/bin/bash' > "$ROOT_MNT/etc/rc.local"
  echo 'bash /home/pi/hone-first-boot.sh &' >> "$ROOT_MNT/etc/rc.local"
  echo 'exit 0' >> "$ROOT_MNT/etc/rc.local"
  chmod +x "$ROOT_MNT/etc/rc.local"
fi

ok "First-boot script installed"

# ─── Unmount ─────────────────────────────────────────────────────
sync
umount "$BOOT_MNT"
umount "$ROOT_MNT"
rmdir "$BOOT_MNT" "$ROOT_MNT"

echo ""
echo "============================================================"
echo "  HONE Nebra SD Card Ready!"
echo "============================================================"
echo ""
echo "  1. Remove the SD card from your PC"
echo "  2. Insert it into the Nebra Indoor hotspot"
echo "  3. Connect Ethernet + power"
echo "  4. Wait 5-10 minutes for first boot"
echo "     (Node.js install + HONE clone + npm install)"
echo "  5. SSH in: ssh pi@<nebra-ip>"
echo "     Password: $NEBRA_PASS"
echo "  6. Check status: sudo systemctl status hone-nebra"
echo ""
echo "  The device registers as shindevlin's LoRa gateway"
echo "  and starts earning clock + IoT rewards immediately."
echo ""
echo "============================================================"

#!/bin/bash
# Install HONE systemd services for the current user
# Run: bash systemd/install.sh

USER=$(whoami)
UNIT_DIR="$HOME/.config/systemd/user"
mkdir -p "$UNIT_DIR"

echo "[hone] Installing systemd services for $USER..."

for service in hone-miner hone-clock hone-explorer honescan-tunnel; do
  # Replace %i with username and %h with home
  sed "s/%i/$USER/g; s|%h|$HOME|g" "systemd/$service.service" > "$UNIT_DIR/$service.service"
  echo "  Installed $service"
done

systemctl --user daemon-reload
systemctl --user enable hone-clock hone-miner hone-explorer honescan-tunnel

echo ""
echo "[hone] Services installed. Commands:"
echo "  systemctl --user start hone-clock hone-miner hone-explorer honescan-tunnel"
echo "  systemctl --user status hone-miner"
echo "  journalctl --user -u hone-miner -f"
echo ""
echo "[hone] To start on boot (even without login):"
echo "  sudo loginctl enable-linger $USER"

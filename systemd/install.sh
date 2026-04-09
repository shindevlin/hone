#!/bin/bash
# Install BTCPC systemd services for the current user
# Run: bash systemd/install.sh

USER=$(whoami)
UNIT_DIR="$HOME/.config/systemd/user"
mkdir -p "$UNIT_DIR"

echo "[btcpc] Installing systemd services for $USER..."

for service in btcpc-miner btcpc-clock btcpc-explorer; do
  # Replace %i with username and %h with home
  sed "s/%i/$USER/g; s|%h|$HOME|g" "systemd/$service.service" > "$UNIT_DIR/$service.service"
  echo "  Installed $service"
done

systemctl --user daemon-reload
systemctl --user enable btcpc-clock btcpc-miner btcpc-explorer

echo ""
echo "[btcpc] Services installed. Commands:"
echo "  systemctl --user start btcpc-clock btcpc-miner btcpc-explorer"
echo "  systemctl --user status btcpc-miner"
echo "  journalctl --user -u btcpc-miner -f"
echo ""
echo "[btcpc] To start on boot (even without login):"
echo "  sudo loginctl enable-linger $USER"

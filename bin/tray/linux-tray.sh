#!/bin/bash

# BTCPC Linux System Tray
# Uses yad (GTK dialog) for AppIndicator tray icon
# Works on GNOME, KDE, XFCE, Budgie, Cinnamon
#
# Install: sudo apt install yad
# The icon appears in the system tray / top panel
# Left-click: opens Settings page in browser
# Right-click: menu with status, settings, explorer, pause/resume, quit

EXPLORER_URL="http://localhost:4242"
SETTINGS_URL="$EXPLORER_URL/settings"
PIPE="/tmp/btcpc-tray-pipe"

# Check yad
if ! command -v yad &>/dev/null; then
  echo "[BTCPC Tray] yad not found. Install: sudo apt install yad"
  echo "[BTCPC Tray] Falling back to notify-send only"

  # Notification-only fallback
  notify-send --app-name=BTCPC "BTCPC Miner" "Mining active. Install yad for system tray icon." 2>/dev/null
  while true; do
    sleep 60
    STATUS=$(curl -s "$EXPLORER_URL/api/stats" 2>/dev/null)
    if echo "$STATUS" | grep -q "state_root"; then
      EPOCH=$(echo "$STATUS" | python3 -c "import sys,json; print(json.load(sys.stdin).get('current_epoch',0))" 2>/dev/null)
      # Only notify on updates
      UPDATE_CHECK=$(curl -s -H "x-bot-key: ${BOT_API_KEY}" "http://localhost:3100/api/bot/update-status" 2>/dev/null)
      if echo "$UPDATE_CHECK" | grep -q '"pending":true'; then
        notify-send --urgency=normal --app-name=BTCPC "BTCPC Update" "New version available. Update to keep earning." 2>/dev/null
      fi
    fi
  done
  exit 0
fi

# Create named pipe for yad communication
rm -f "$PIPE"
mkfifo "$PIPE"

# Trap cleanup
cleanup() {
  rm -f "$PIPE"
  kill $(jobs -p) 2>/dev/null
  exit 0
}
trap cleanup EXIT INT TERM

# Start yad notification icon
exec 3<>"$PIPE"

yad --notification \
  --listen \
  --text="BTCPC Miner" \
  --command="xdg-open $SETTINGS_URL" \
  --icon-size=22 \
  <&3 &
YAD_PID=$!

# Update loop
while kill -0 $YAD_PID 2>/dev/null; do
  # Get status from explorer API
  STATUS_JSON=$(curl -s "$EXPLORER_URL/api/stats" 2>/dev/null)

  if echo "$STATUS_JSON" | grep -q "current_epoch"; then
    EPOCH=$(echo "$STATUS_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('current_epoch',0))" 2>/dev/null)
    MINED=$(echo "$STATUS_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(f\"{d.get('total_mined',0):.0f}\")" 2>/dev/null)
    BLOCKS=$(echo "$STATUS_JSON" | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('blocks_on_disk',0))" 2>/dev/null)

    TOOLTIP="BTCPC Mining — Epoch $EPOCH | $MINED BTCPC mined | $BLOCKS blocks"
    echo "tooltip:$TOOLTIP" >&3

    # Build menu
    MENU="Settings!xdg-open $SETTINGS_URL"
    MENU="$MENU|Explorer!xdg-open $EXPLORER_URL"
    MENU="$MENU|---"
    MENU="$MENU|Pause Mining!curl -s -X POST $EXPLORER_URL/settings -d action=pause"
    MENU="$MENU|Resume Mining!curl -s -X POST $EXPLORER_URL/settings -d action=auto"
    MENU="$MENU|---"
    MENU="$MENU|Quit!kill $$"
    echo "menu:$MENU" >&3
  else
    echo "tooltip:BTCPC Miner — offline" >&3
  fi

  # Check for updates
  UPDATE_CHECK=$(curl -s -H "x-bot-key: ${BOT_API_KEY}" "http://localhost:3100/api/bot/update-status" 2>/dev/null)
  if echo "$UPDATE_CHECK" | grep -q '"pending":true'; then
    VERSION=$(echo "$UPDATE_CHECK" | python3 -c "import sys,json; u=json.load(sys.stdin).get('update',{}); print(u.get('version','?'))" 2>/dev/null)
    notify-send --urgency=normal --app-name=BTCPC \
      "BTCPC Update v$VERSION" \
      "New version available. Update to keep earning." 2>/dev/null
  fi

  sleep 30
done

#!/bin/bash
# Start the full HONE marketing stack:
#   1. Dashboard server (port 7979)
#   2. Geckodriver per persona profile (port 4444)
# Timers (drafter at 6am, poster every 15min) run via systemd.

set -e
REPO="$(cd "$(dirname "$0")/.." && pwd)"
PROFILES_DIR="$REPO/firefox-profiles"

echo "=== HONE Marketing Stack ==="
echo ""

# 1. Dashboard server via systemd (auto-restarts on crash)
systemctl --user daemon-reload
systemctl --user enable --now hone-marketing-dashboard.service
sleep 1
if systemctl --user is-active --quiet hone-marketing-dashboard.service; then
  echo "✓ Dashboard server   http://localhost:7979"
else
  echo "✗ Dashboard server failed to start"
  systemctl --user status hone-marketing-dashboard.service --no-pager | tail -5
fi

# 2. ZeroClaw binary — build if missing
ZEROCLAW_BIN="$HOME/repos/zeroclaw/target/release/zeroclaw"
if [ ! -f "$ZEROCLAW_BIN" ]; then
  echo ""
  echo "ZeroClaw binary not found — building (this takes ~2 min)..."
  bash "$REPO/scripts/build-zeroclaw.sh"
fi
echo "✓ ZeroClaw binary    $ZEROCLAW_BIN"

# 3. Geckodriver for poster automation
fuser -k 4444/tcp 2>/dev/null || true
sleep 1
mkdir -p "$PROFILES_DIR/shindevlin" "$PROFILES_DIR/natoshisakamoto"
geckodriver --port 4444 --profile-root "$PROFILES_DIR/shindevlin" --log warn &
echo "$!" > "$REPO/.geckodriver.pid"
sleep 1
echo "✓ Geckodriver        http://localhost:4444"

# 4. Timers
systemctl --user enable --now hone-marketing-drafter.timer 2>/dev/null || true
systemctl --user enable --now hone-marketing-poster.timer  2>/dev/null || true
echo "✓ Drafter timer      daily 6am"
echo "✓ Poster timer       every 15min"

echo ""
echo "=== Setup checklist ==="
for PERSONA in shindevlin natoshisakamoto; do
  PROFILE="$PROFILES_DIR/$PERSONA"
  if [ -f "$PROFILE/lock" ] || find "$PROFILE" -name "cookies.sqlite" 2>/dev/null | grep -q .; then
    echo "✓ $PERSONA sessions  saved"
  else
    echo "✗ $PERSONA sessions  NOT SET UP"
    echo "  → Run: bash scripts/setup-accounts.sh $PERSONA"
  fi
done

HONE_REPO="$(cd "$REPO/.." && pwd)"
if grep -q "^TELEGRAM_CHAT_ID=your-chat-id" "$HONE_REPO/.env" 2>/dev/null; then
  echo "✗ Telegram CHAT_ID   not configured"
  echo "  → Message @userinfobot on Telegram, paste your chat_id into .env"
else
  echo "✓ Telegram alerts    configured"
fi

echo ""
echo "Open: http://localhost:7979"

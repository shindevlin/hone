#!/bin/bash
# Run ZeroClaw poster — posts approved content via Firefox (geckodriver), inference through BTCPC chain
set -e

REPO="$(cd "$(dirname "$0")/.." && pwd)"
BTCPC_REPO="$(cd "$REPO/.." && pwd)"
ZEROCLAW_BIN="${ZEROCLAW_BIN:-$HOME/repos/zeroclaw/target/release/zeroclaw}"
CONFIG_TEMPLATE="$REPO/zeroclaw/poster/config.toml"
CONFIG_LIVE="$REPO/zeroclaw/poster/.config.live.toml"
PROMPT_FILE="$REPO/zeroclaw/poster/prompt.md"
PROFILES_DIR="$REPO/firefox-profiles"
WEBDRIVER_PORT=4444

# Load BTCPC API key from .env
if [ -f "$BTCPC_REPO/.env" ]; then
  BTCPC_API_KEY=$(grep '^BTCPC_RELAY_API_KEY=' "$BTCPC_REPO/.env" | cut -d= -f2- | tr -d '"')
fi
if [ -z "$BTCPC_API_KEY" ]; then
  echo "ERROR: BTCPC_RELAY_API_KEY not found in $BTCPC_REPO/.env"
  exit 1
fi

# Check dashboard server is running
if ! curl -sf http://localhost:7979/api/posts > /dev/null 2>&1; then
  echo "ERROR: Dashboard server not running. Start it first: $REPO/scripts/start.sh"
  exit 1
fi

# Check if there are approved posts
APPROVED_COUNT=$(curl -sf "http://localhost:7979/api/posts?status=approved" | python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo 0)
if [ "$APPROVED_COUNT" -eq 0 ]; then
  echo "No approved posts to publish. Nothing to do."
  exit 0
fi

# Write live config with real API key injected
sed "s|BTCPC_API_KEY_HERE|$BTCPC_API_KEY|g" "$CONFIG_TEMPLATE" > "$CONFIG_LIVE"

# Determine which persona's posts need to go out
# (run once per persona so each gets its own Firefox profile/session)
PERSONAS=$(curl -sf "http://localhost:7979/api/posts?status=approved" | \
  python3 -c "import sys,json; posts=json.load(sys.stdin); print('\n'.join(sorted(set(p['persona'] for p in posts))))")

for PERSONA in $PERSONAS; do
  PERSONA_COUNT=$(curl -sf "http://localhost:7979/api/posts?status=approved&persona=$PERSONA" | \
    python3 -c "import sys,json; print(len(json.load(sys.stdin)))" 2>/dev/null || echo 0)
  [ "$PERSONA_COUNT" -eq 0 ] && continue

  PROFILE_DIR="$PROFILES_DIR/$PERSONA"
  if [ ! -d "$PROFILE_DIR" ]; then
    echo "WARNING: No Firefox profile for $PERSONA. Run scripts/setup-accounts.sh $PERSONA first."
    continue
  fi

  echo "Starting geckodriver for $PERSONA..."
  fuser -k "${WEBDRIVER_PORT}/tcp" 2>/dev/null || true
  sleep 1
  geckodriver --port "$WEBDRIVER_PORT" --profile-root "$PROFILE_DIR" --log warn &
  GD_PID=$!
  sleep 2

  echo "Running ZeroClaw poster for $PERSONA ($PERSONA_COUNT posts)..."
  ZEROCLAW_CONFIG_DIR="$(dirname "$CONFIG_LIVE")" \
    "$ZEROCLAW_BIN" agent \
    --message "$(cat "$PROMPT_FILE") Process only posts for persona: $PERSONA"

  kill "$GD_PID" 2>/dev/null || true
  sleep 1
done

rm -f "$CONFIG_LIVE"
echo "Posting complete."

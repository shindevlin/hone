#!/bin/bash
# Run ZeroClaw drafter — generates new post drafts, inference goes through HONE chain
set -e

REPO="$(cd "$(dirname "$0")/.." && pwd)"
HONE_REPO="$(cd "$REPO/.." && pwd)"
ZEROCLAW_BIN="${ZEROCLAW_BIN:-$HOME/.cargo/bin/zeroclaw}"
CONFIG_TEMPLATE="$REPO/zeroclaw/drafter/config.toml"
CONFIG_LIVE="$REPO/zeroclaw/drafter/.config.live.toml"
PROMPT_FILE="$REPO/zeroclaw/drafter/prompt.md"

# Load HONE API key from .env
if [ -f "$HONE_REPO/.env" ]; then
  HONE_API_KEY=$(grep '^HONE_RELAY_API_KEY=' "$HONE_REPO/.env" | cut -d= -f2- | tr -d '"')
fi
if [ -z "$HONE_API_KEY" ]; then
  echo "ERROR: HONE_RELAY_API_KEY not found in $HONE_REPO/.env"
  exit 1
fi

# Check dashboard server is running
if ! curl -sf http://localhost:7979/api/posts > /dev/null 2>&1; then
  echo "ERROR: Dashboard server not running. Start it first: $REPO/scripts/start.sh"
  exit 1
fi

# Write live config with real API key injected
sed "s|HONE_API_KEY_HERE|$HONE_API_KEY|g" "$CONFIG_TEMPLATE" > "$CONFIG_LIVE"

echo "Running ZeroClaw drafter (inference → HONE chain)..."
ZEROCLAW_CONFIG_DIR="$(dirname "$CONFIG_LIVE")" \
  "$ZEROCLAW_BIN" agent --message "$(cat "$PROMPT_FILE")"

rm -f "$CONFIG_LIVE"
echo "Drafting complete. Open http://localhost:7979 to review."

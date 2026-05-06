#!/bin/bash
# Start geckodriver for ZeroClaw poster automation.
# Each persona gets its own Firefox profile so sessions persist across runs.
set -e

REPO="$(cd "$(dirname "$0")/.." && pwd)"
PROFILES_DIR="$REPO/firefox-profiles"
PERSONA="${1:-shindevlin}"   # pass persona as arg: shindevlin | natoshisakamoto
PORT="${2:-4444}"
PROFILE_DIR="$PROFILES_DIR/$PERSONA"

mkdir -p "$PROFILE_DIR"

# Kill any existing geckodriver on this port
fuser -k "${PORT}/tcp" 2>/dev/null || true
sleep 1

echo "Starting geckodriver for persona=$PERSONA on port=$PORT"
echo "  Firefox profile: $PROFILE_DIR"

# Start geckodriver with the persona's Firefox profile
geckodriver \
  --port "$PORT" \
  --profile-root "$PROFILE_DIR" \
  --log warn \
  &

GD_PID=$!
echo "$GD_PID" > "$REPO/.geckodriver-${PERSONA}.pid"
echo "geckodriver started (pid=$GD_PID)"
echo "Open http://localhost:$PORT for WebDriver endpoint"

#!/usr/bin/env bash
# Daily health-check runner: runs the healthcheck against the live node, saves a
# dated report, posts a summary to the agent channel, and updates the scoreboard
# health line. Intended to be run once a day by cron / systemd timer / scheduled task.
#
# Env:
#   HONE_HEALTHCHECK_URL  — node to probe (default http://127.0.0.1:4242)
#   BRIDGE_DIR             — path to pc-agent-bridge (default /mnt/x/pc-agent-bridge)
#   CHANNEL_BIN            — channel binary (default /mnt/x/comm-channel/target/release/channel.exe)
#   REPORTS_DIR            — where to save dated reports (default /mnt/x/reports/health)
set -uo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
URL="${HONE_HEALTHCHECK_URL:-http://127.0.0.1:4242}"
REPORTS_DIR="${REPORTS_DIR:-$REPO/reports/health}"
BRIDGE_DIR="${BRIDGE_DIR:-/mnt/x/pc-agent-bridge}"
CHANNEL_BIN="${CHANNEL_BIN:-/mnt/x/comm-channel/target/release/channel.exe}"

DATE="$(date -u +%Y-%m-%d)"
mkdir -p "$REPORTS_DIR"
MD="$REPORTS_DIR/healthcheck-$DATE.md"
JSON="$REPORTS_DIR/healthcheck-$DATE.json"

# Run it (read-only; never signs). Capture the report + exit code.
node "$REPO/scripts/daily-healthcheck.mjs" --url "$URL" --json "$JSON" > "$MD" 2>&1
CODE=$?

# Extract the one-line summary for the channel/scoreboard.
SUMMARY="$(grep -m1 'Summary:' "$MD" | sed 's/.*Summary:\*\* //' )"
FOCUS="$(awk '/## Where to focus/{f=1;next} f&&/^[0-9]/{print; c++} c>=2{exit}' "$MD" | tr '\n' ' ')"

echo "healthcheck $DATE: $SUMMARY (exit $CODE)"
echo "report: $MD"

# Post to the agent channel so both agents + the human see it (broadcast).
if [ -x "$CHANNEL_BIN" ] && [ -d "$BRIDGE_DIR" ]; then
  ( cd "$BRIDGE_DIR" && "$CHANNEL_BIN" send --to all --type status \
    "DAILY HEALTHCHECK $DATE — $SUMMARY. Focus: $FOCUS Full report: reports/health/healthcheck-$DATE.md $( [ "$CODE" = "2" ] && echo '⛔ CRITICAL breakage — see report' )" ) \
    2>/dev/null && echo "posted to channel"
fi

exit $CODE

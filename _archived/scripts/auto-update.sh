#!/bin/bash
# BTCPC Miner Auto-Update
# Checks for new commits, pulls, restarts miner if changed.
# Run via cron every 5 minutes:
#   */5 * * * * /home/$USER/repos/btcpc/scripts/auto-update.sh >> /tmp/btcpc-update.log 2>&1

set -euo pipefail

BTCPC_DIR="${BTCPC_DIR:-$HOME/repos/btcpc}"
SERVICE_NAME="${BTCPC_SERVICE:-btcpc-mine}"
BRANCH="${BTCPC_BRANCH:-main}"

cd "$BTCPC_DIR"

# Fetch remote
git fetch origin "$BRANCH" --quiet

# Compare local vs remote
LOCAL=$(git rev-parse HEAD)
REMOTE=$(git rev-parse "origin/$BRANCH")

if [ "$LOCAL" = "$REMOTE" ]; then
  exit 0
fi

echo "[$(date)] Updating: $LOCAL -> $REMOTE"

# Pull
git pull origin "$BRANCH" --quiet

# Reinstall deps if package.json changed
if git diff "$LOCAL" "$REMOTE" --name-only | grep -q "package.json"; then
  echo "[$(date)] package.json changed, running npm install"
  npm install --production --quiet
fi

# Restart miner
if systemctl is-active "$SERVICE_NAME" >/dev/null 2>&1; then
  echo "[$(date)] Restarting $SERVICE_NAME"
  sudo systemctl restart "$SERVICE_NAME"
else
  echo "[$(date)] Service $SERVICE_NAME not active, skipping restart"
fi

echo "[$(date)] Updated to $(git rev-parse --short HEAD)"

#!/usr/bin/env bash
# Polls the shared repo every 30s and prints any new messages from Beastly.
REPO=/mnt/btcpc-storage/repos/btcpc
INBOX=$REPO/bridge/beastly
SEEN_FILE=/tmp/btcpc-bridge-seen

mkdir -p "$INBOX"
touch "$SEEN_FILE"

echo "[bridge] watching for Beastly messages (every 30s)..."

while true; do
  git -C "$REPO" pull --quiet origin flipper/full-pipeline 2>/dev/null
  for f in "$INBOX"/*.md; do
    [ -f "$f" ] || continue
    fname=$(basename "$f")
    if ! grep -qxF "$fname" "$SEEN_FILE"; then
      echo ""
      echo "════════════════════════════════════════"
      echo "[bridge] NEW MESSAGE FROM BEASTLY: $fname"
      echo "════════════════════════════════════════"
      cat "$f"
      echo "════════════════════════════════════════"
      echo "$fname" >> "$SEEN_FILE"
    fi
  done
  sleep 30
done

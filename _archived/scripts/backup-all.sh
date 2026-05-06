#!/bin/bash
# BTCPC ecosystem backup — all repos + chain data
# Run daily via cron: 0 3 * * * /home/ubuntclaw/repos/btcpc/scripts/backup-all.sh

BACKUP_DIR="/home/ubuntclaw/backups"
DATE=$(date +%Y-%m-%d)
BACKUP_PATH="$BACKUP_DIR/$DATE"
LOG="$BACKUP_DIR/backup.log"

mkdir -p "$BACKUP_PATH"

echo "[$DATE] Backup started" >> "$LOG"

# ── Repos (code only, skip node_modules) ──
REPOS=(
  btcpc btcpcbot btcpcwalletbot alertbot
  bullship nsfwotica brutus11 ursOS realfake redaktly
  BusWingSpread betchu_bot counselflow spirit-of-ngu itsurs waitlyfi
)

for repo in "${REPOS[@]}"; do
  src="/home/ubuntclaw/repos/$repo"
  if [ -d "$src" ]; then
    rsync -a --exclude='node_modules' --exclude='.git' "$src/" "$BACKUP_PATH/$repo/"
  fi
done
echo "[$DATE]   Repos backed up: ${#REPOS[@]}" >> "$LOG"

# ── .env and .envbtcpc files (secrets) ──
mkdir -p "$BACKUP_PATH/secrets"
for repo in "${REPOS[@]}"; do
  src="/home/ubuntclaw/repos/$repo"
  [ -f "$src/.env" ] && cp "$src/.env" "$BACKUP_PATH/secrets/${repo}.env"
  [ -f "$src/.envbtcpc" ] && cp "$src/.envbtcpc" "$BACKUP_PATH/secrets/${repo}.envbtcpc"
done
echo "[$DATE]   Secrets backed up" >> "$LOG"

# ── MongoDB dump ──
docker exec urs-node-vault-mongodb mongodump --uri="mongodb://root:example@localhost:27017/btcpc?authSource=admin" --out="/tmp/mongodump-$DATE" 2>/dev/null
if [ -d "/tmp/mongodump-$DATE" ]; then
  docker cp "urs-node-vault-mongodb:/tmp/mongodump-$DATE" "$BACKUP_PATH/mongodb/"
  docker exec urs-node-vault-mongodb rm -rf "/tmp/mongodump-$DATE"
  echo "[$DATE]   MongoDB dumped" >> "$LOG"
fi

# ── Inference results ──
for repo in "${REPOS[@]}"; do
  src="/home/ubuntclaw/repos/$repo/.btcpc-inference"
  if [ -d "$src" ]; then
    mkdir -p "$BACKUP_PATH/inference/$repo"
    cp -r "$src/"* "$BACKUP_PATH/inference/$repo/" 2>/dev/null
  fi
done
echo "[$DATE]   Inference results backed up" >> "$LOG"

# ── Cleanup old backups (keep 30 days) ──
find "$BACKUP_DIR" -maxdepth 1 -type d -mtime +30 -exec rm -rf {} \;

echo "[$DATE] Backup complete: $BACKUP_PATH" >> "$LOG"
echo "Backup complete: $BACKUP_PATH"

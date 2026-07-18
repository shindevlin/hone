#!/bin/bash
# HONE Node Environment Setup
# Usage: bash scripts/setup-node-env.sh [username]
# Mining nodes only need the posting key. Active/owner keys should stay off-node.

set -euo pipefail
USERNAME="${1:-}"

if [ -z "$USERNAME" ]; then
  echo "Do you already have a HONE account? [y/N]"
  read -r HAS_ACCOUNT
  echo "Enter HONE username for this node:"
  read -r USERNAME
fi

ENV_FILE=".env"

if [ -z "$USERNAME" ]; then
  echo "Username cannot be empty."
  exit 1
fi

POSTING_KEY=""
if [ "${HAS_ACCOUNT:-y}" = "y" ] || [ "${HAS_ACCOUNT:-y}" = "Y" ] || [ "${HAS_ACCOUNT:-yes}" = "yes" ]; then
  echo "Paste HONE posting private key for mining (64 hex chars)."
  echo "Do not paste owner or active keys on mining nodes."
  read -rs POSTING_KEY
  echo
  if [ -n "$POSTING_KEY" ] && ! echo "$POSTING_KEY" | grep -Eq '^[0-9a-fA-F]{64}$'; then
    echo "Posting key must be 64 hex characters."
    exit 1
  fi
else
  echo "Create the account on the website or bot first, save the full wallet export,"
  echo "then rerun this script with the posting private key. Staking is managed on the website."
  exit 0
fi

# Generate JWT secret
JWT_SECRET=$(openssl rand -hex 32)

cat > "$ENV_FILE" << ENVFILE
# HONE Node Configuration — $USERNAME
NODE_ENV=production
PORT=3000

# Miner identity. Mining uses posting key only.
HONE_MINER=$USERNAME
HONE_POSTING_KEY=$POSTING_KEY

# Roles (all = miner + clock + storage + api)
HONE_ROLES=all

# P2P Network
P2P_PORT=6942
HONE_SEED_PEERS=wss://hone-relay.shindevlin.workers.dev/ws
HONE_RELAY_URL=wss://hone-relay.shindevlin.workers.dev/ws

# Security
JWT_SECRET=$JWT_SECRET
JWT_EXPIRES_IN=7d

# Ollama (local inference)
OLLAMA_URL=http://localhost:11434

# MongoDB (optional — disabled by default post-Phase F)
HONE_MONGO_MODE=disabled

# Storage
HONE_STORAGE_CAPACITY_GB=10
ENVFILE

# Secure the file
chmod 600 "$ENV_FILE"

echo ""
echo ".env written for '$USERNAME'"
echo "   Posting key: set (not displayed)"
echo "   JWT Secret: ${JWT_SECRET:0:16}..."
echo "   Roles: all"
echo "   File permissions: 600 (owner only)"
echo ""
echo "   No mnemonic, owner key, or active key on this machine."
echo "   Owner key rotates keys. Active key moves tokens. Posting key mines."
echo ""
echo "Next: node bin/hone-all"

#!/bin/bash
# BTCPC Node Environment Setup
# Usage: bash scripts/setup-node-env.sh <username>
# Writes the correct .env for the given miner account

set -euo pipefail
USERNAME="${1:-}"

if [ -z "$USERNAME" ]; then
  echo "Usage: bash scripts/setup-node-env.sh <username>"
  echo "  e.g.: bash scripts/setup-node-env.sh shindevlin"
  exit 1
fi

ENV_FILE=".env"

# Check if mnemonic is provided via env or we need to ask
MNEMONIC="${BTCPC_MNEMONIC:-}"
if [ -z "$MNEMONIC" ]; then
  echo "Enter the 12-word mnemonic for '$USERNAME':"
  read -r MNEMONIC
fi

if [ -z "$MNEMONIC" ]; then
  echo "No mnemonic provided. The account will be created without keys."
  echo "(You can add BTCPC_MNEMONIC= to .env later and restart)"
fi

# Generate JWT secret if not already in .env
JWT_SECRET=$(openssl rand -hex 32)

cat > "$ENV_FILE" << ENVFILE
# BTCPC Node Configuration — $USERNAME
NODE_ENV=production
PORT=3000

# Miner identity
BTCPC_MINER=$USERNAME
BTCPC_MNEMONIC=$MNEMONIC

# Roles (all = miner + clock + storage + api)
BTCPC_ROLES=all

# P2P Network
P2P_PORT=6942
BTCPC_SEED_PEERS=wss://btcpc-relay.shindevlin.workers.dev/ws
BTCPC_RELAY_URL=wss://btcpc-relay.shindevlin.workers.dev/ws

# Security
JWT_SECRET=$JWT_SECRET
JWT_EXPIRES_IN=7d

# Ollama (local inference)
OLLAMA_URL=http://localhost:11434

# MongoDB (optional — disabled by default post-Phase F)
BTCPC_MONGO_MODE=disabled

# Storage
BTCPC_STORAGE_CAPACITY_GB=10
ENVFILE

echo "✅ .env written for '$USERNAME'"
echo "   Mnemonic: ${MNEMONIC:0:20}..."
echo "   JWT Secret: ${JWT_SECRET:0:16}..."
echo "   Roles: all"
echo ""
echo "Next: node bin/btcpc-all"

#!/bin/bash
# BTCPC Node Environment Setup
# Usage: bash scripts/setup-node-env.sh <username>
# Writes the correct .env with username + password only.
# The mnemonic stays with the USER — never on the machine.

set -euo pipefail
USERNAME="${1:-}"

if [ -z "$USERNAME" ]; then
  echo "Usage: bash scripts/setup-node-env.sh <username>"
  echo "  e.g.: bash scripts/setup-node-env.sh shindevlin"
  exit 1
fi

ENV_FILE=".env"

# Get password (hidden input)
echo "Enter the password for '$USERNAME':"
read -rs PASSWORD
echo "Confirm password:"
read -rs PASSWORD_CONFIRM

if [ "$PASSWORD" != "$PASSWORD_CONFIRM" ]; then
  echo "❌ Passwords don't match. Try again."
  exit 1
fi

if [ -z "$PASSWORD" ]; then
  echo "❌ Password cannot be empty."
  exit 1
fi

# Generate JWT secret
JWT_SECRET=$(openssl rand -hex 32)

cat > "$ENV_FILE" << ENVFILE
# BTCPC Node Configuration — $USERNAME
NODE_ENV=production
PORT=3000

# Miner identity (username only — mnemonic stays with the user, not on disk)
BTCPC_MINER=$USERNAME
BTCPC_PASSWORD=$PASSWORD

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

# Secure the file
chmod 600 "$ENV_FILE"

echo ""
echo "✅ .env written for '$USERNAME'"
echo "   Password: set (not displayed)"
echo "   JWT Secret: ${JWT_SECRET:0:16}..."
echo "   Roles: all"
echo "   File permissions: 600 (owner only)"
echo ""
echo "   ⚠️  No mnemonic on this machine."
echo "   The mnemonic stays with the user (paper/Signal/Saved Messages)."
echo "   This node authenticates via username + password."
echo ""
echo "Next: node bin/btcpc-all"

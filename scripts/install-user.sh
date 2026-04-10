#!/usr/bin/env bash
set -euo pipefail

USERNAME="${1:-}"
REPO_URL="${BTCPC_REPO_URL:-https://github.com/shindevlin/btcpc.git}"
INSTALL_DIR="${BTCPC_INSTALL_DIR:-$HOME/btcpc}"
MONGO_CONTAINER="${BTCPC_MONGO_CONTAINER:-btcpc-mongo}"
MONGO_URI="mongodb://root:example@localhost:27017/btcpc?authSource=admin"

if [[ -z "$USERNAME" ]]; then
  echo "Usage: bash scripts/install-user.sh <username>"
  exit 1
fi

if [[ ! "$USERNAME" =~ ^[a-z0-9][a-z0-9-]{2,19}$ ]]; then
  echo "Username must be 3-20 chars, lowercase letters, numbers, and hyphens only."
  exit 1
fi

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "This installer currently supports Linux only."
  exit 1
fi

if ! command -v sudo >/dev/null 2>&1; then
  echo "sudo is required."
  exit 1
fi

echo "Installing BTCPC for user: $USERNAME"

sudo apt-get update
sudo apt-get install -y curl git ca-certificates docker.io

if ! command -v node >/dev/null 2>&1 || [[ "$(node -v | sed 's/^v//;s/\\..*//')" -lt 20 ]]; then
  curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
  sudo apt-get install -y nodejs
fi

sudo systemctl enable --now docker

if ! sudo docker ps -a --format '{{.Names}}' | grep -qx "$MONGO_CONTAINER"; then
  sudo docker run -d \
    --name "$MONGO_CONTAINER" \
    -p 27017:27017 \
    -e MONGO_INITDB_ROOT_USERNAME=root \
    -e MONGO_INITDB_ROOT_PASSWORD=example \
    --restart unless-stopped \
    mongo:7
else
  sudo docker start "$MONGO_CONTAINER" >/dev/null || true
fi

if [[ ! -d "$INSTALL_DIR/.git" ]]; then
  git clone "$REPO_URL" "$INSTALL_DIR"
else
  git -C "$INSTALL_DIR" pull --ff-only
fi

cd "$INSTALL_DIR"
npm install

JWT_SECRET_VALUE="$(openssl rand -hex 32)"

cat > .env <<EOF
PORT=3000
NODE_ENV=production
MONGODB_URI=$MONGO_URI
JWT_SECRET=$JWT_SECRET_VALUE
JWT_EXPIRES_IN=7d
RATE_LIMIT_WINDOW_MS=900000
RATE_LIMIT_MAX_REQUESTS=100
EOF

BTCPC_MINER="$USERNAME" \
BTCPC_CLOCK_ACCOUNT="$USERNAME" \
MONGODB_URI="$MONGO_URI" \
node bin/btcpc-setup --yes --username "$USERNAME" --skip-ollama

echo
echo "BTCPC is installed."
echo "Username: $USERNAME"
echo "Explorer: http://localhost:4242"
echo "API: http://localhost:3000"
echo
echo "This install runs wallet/explorer/clock mode."
echo "If you want mining later, run:"
echo "  cd $INSTALL_DIR && BTCPC_MINER=$USERNAME node bin/btcpc-setup"

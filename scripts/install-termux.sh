#!/data/data/com.termux/files/usr/bin/bash
# BTCPC Termux Installer — one command, zero questions
# Usage: curl -fsSL https://btcpc.net/install-termux.sh | bash

set -e

echo "========================================="
echo "  BTCPC — Bitcoin Proof of Compute"
echo "  Termux Installer (clock node)"
echo "========================================="
echo ""

# Update packages silently
echo "[1/5] Updating Termux packages..."
yes | pkg update -y 2>/dev/null || true
yes | pkg upgrade -y 2>/dev/null || true

# Install Node.js + git
echo "[2/5] Installing Node.js..."
yes | pkg install -y nodejs-lts git 2>/dev/null || true

# Verify node
if ! command -v node >/dev/null 2>&1; then
  echo "ERROR: Node.js failed to install. Try: pkg install nodejs-lts"
  exit 1
fi
echo "  Node $(node --version) installed"

# Clone or update repo
echo "[3/5] Downloading BTCPC..."
if [ -d "$HOME/btcpc" ]; then
  cd "$HOME/btcpc"
  git pull --ff-only 2>/dev/null || true
else
  git clone https://github.com/shindevlin/btcpc.git "$HOME/btcpc"
  cd "$HOME/btcpc"
fi

# Install dependencies
echo "[4/5] Installing dependencies..."
npm install --production 2>/dev/null

# Setup account
echo "[5/5] Setting up account..."
if [ ! -f "$HOME/btcpc/.env" ]; then
  echo ""
  echo "Enter your BTCPC username:"
  read -r BTCPC_USER
  echo ""
  echo "Enter a password:"
  read -rs BTCPC_PASS
  echo ""

  cat > "$HOME/btcpc/.env" << ENVEOF
BTCPC_MINER=$BTCPC_USER
BTCPC_PASSWORD=$BTCPC_PASS
BTCPC_ROLES=clock
BTCPC_MONGO_MODE=disabled
PORT=3000
ENVEOF

  echo "  Account configured: $BTCPC_USER (clock node)"
else
  echo "  .env already exists, keeping current config"
fi

# Prevent Android from killing Termux
termux-wake-lock 2>/dev/null || true

echo ""
echo "========================================="
echo "  BTCPC installed!"
echo ""
echo "  Start:   cd ~/btcpc && node bin/btcpc-all"
echo "  Or just:  btcpc"
echo "========================================="

# Create shortcut command
mkdir -p "$HOME/.shortcuts" 2>/dev/null || true
cat > "$PREFIX/bin/btcpc" << 'CMDEOF'
#!/data/data/com.termux/files/usr/bin/bash
cd "$HOME/btcpc" && exec node bin/btcpc-all
CMDEOF
chmod +x "$PREFIX/bin/btcpc"

echo ""
echo "Type 'btcpc' to start your node."

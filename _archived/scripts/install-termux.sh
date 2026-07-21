#!/data/data/com.termux/files/usr/bin/bash
# HONE Termux Installer — one command, zero questions
# Usage: curl -fsSL https://hone.net/install-termux.sh | bash

set -e

echo "========================================="
echo "  HONE — Bitcoin Proof of Compute"
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
echo "[3/5] Downloading HONE..."
if [ -d "$HOME/hone" ]; then
  cd "$HOME/hone"
  git pull --ff-only 2>/dev/null || true
else
  git clone https://github.com/shindevlin/hone.git "$HOME/hone"
  cd "$HOME/hone"
fi

# Install dependencies
echo "[4/5] Installing dependencies..."
npm install --production 2>/dev/null

# Setup account
echo "[5/5] Setting up account..."
if [ ! -f "$HOME/hone/.env" ]; then
  echo ""
  echo "Enter your HONE username:"
  read -r HONE_USER
  echo ""
  echo "Enter a password:"
  read -rs HONE_PASS
  echo ""

  cat > "$HOME/hone/.env" << ENVEOF
HONE_MINER=$HONE_USER
HONE_PASSWORD=$HONE_PASS
HONE_ROLES=clock
HONE_MONGO_MODE=disabled
PORT=3000
ENVEOF

  echo "  Account configured: $HONE_USER (clock node)"
else
  echo "  .env already exists, keeping current config"
fi

# Prevent Android from killing Termux
termux-wake-lock 2>/dev/null || true

echo ""
echo "========================================="
echo "  HONE installed!"
echo ""
echo "  Start:   cd ~/hone && node bin/hone-all"
echo "  Or just:  hone"
echo "========================================="

# Create shortcut command
mkdir -p "$HOME/.shortcuts" 2>/dev/null || true
cat > "$PREFIX/bin/hone" << 'CMDEOF'
#!/data/data/com.termux/files/usr/bin/bash
cd "$HOME/hone" && exec node bin/hone-all
CMDEOF
chmod +x "$PREFIX/bin/hone"

echo ""
echo "Type 'hone' to start your node."

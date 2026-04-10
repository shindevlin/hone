#!/data/data/com.termux/files/usr/bin/bash
# BTCPC Android Clock Node Installer
# For Termux on Android: install Termux from F-Droid first
# Usage: curl https://btcpc.net/android.sh | bash

set -e

ORANGE='\033[38;5;208m'
GREEN='\033[0;32m'
RESET='\033[0m'

say() { printf "${ORANGE}[btcpc]${RESET} %s\n" "$1"; }
ok()  { printf "${GREEN}[ok]${RESET} %s\n" "$1"; }

cat <<'BANNER'

  ██████╗ ████████╗ ██████╗██████╗  ██████╗
  ██╔══██╗╚══██╔══╝██╔════╝██╔══██╗██╔════╝
  ██████╔╝   ██║   ██║     ██████╔╝██║
  ██╔══██╗   ██║   ██║     ██╔═══╝ ██║
  ██████╔╝   ██║   ╚██████╗██║     ╚██████╗
  ╚═════╝    ╚═╝    ╚═════╝╚═╝      ╚═════╝

  Bitcoin Proof of Compute — Android Clock Installer

BANNER

# Detect Termux
if [ -z "$TERMUX_VERSION" ] && [ ! -d "/data/data/com.termux" ]; then
  echo "ERROR: This script must be run inside Termux on Android."
  echo "Install Termux from F-Droid: https://f-droid.org/packages/com.termux/"
  exit 1
fi

say "Updating Termux packages..."
pkg update -y > /dev/null 2>&1
pkg upgrade -y > /dev/null 2>&1

say "Installing Node.js + curl + wakelock support..."
pkg install -y nodejs-lts curl termux-api termux-services > /dev/null 2>&1

say "Acquiring Termux wake lock (keeps clock running with screen off)..."
if command -v termux-wake-lock >/dev/null 2>&1; then
  termux-wake-lock || true
fi

INSTALL_DIR="$HOME/.btcpc-clock"
mkdir -p "$INSTALL_DIR"

say "Downloading btcpc-clock-lite..."
curl -fsSL https://btcpc.net/clock-lite.js -o "$INSTALL_DIR/clock.js"

# Prompt for account
echo
read -rp "$(printf "${ORANGE}[btcpc]${RESET} Your BTCPC username: ")" BTCPC_ACCOUNT

if [[ ! "$BTCPC_ACCOUNT" =~ ^[a-z0-9][a-z0-9-]{2,19}$ ]]; then
  echo "Invalid username (3-20 chars, lowercase/numbers/hyphens)."
  exit 1
fi

# Save config
cat > "$INSTALL_DIR/config" <<EOF
BTCPC_CLOCK_ACCOUNT=$BTCPC_ACCOUNT
EOF

# Create launcher script
cat > "$INSTALL_DIR/start.sh" <<'EOF'
#!/data/data/com.termux/files/usr/bin/bash
source "$HOME/.btcpc-clock/config"
export BTCPC_CLOCK_ACCOUNT
cd "$HOME/.btcpc-clock"
exec node clock.js
EOF
chmod +x "$INSTALL_DIR/start.sh"

# Set up auto-start via termux-services (runit)
SVC_DIR="$PREFIX/var/service/btcpc-clock"
mkdir -p "$SVC_DIR/log"

cat > "$SVC_DIR/run" <<EOF
#!$PREFIX/bin/sh
exec 2>&1
source $INSTALL_DIR/config
export BTCPC_CLOCK_ACCOUNT
cd $INSTALL_DIR
exec node clock.js
EOF
chmod +x "$SVC_DIR/run"

cat > "$SVC_DIR/log/run" <<EOF
#!$PREFIX/bin/sh
exec logger -t btcpc-clock
EOF
chmod +x "$SVC_DIR/log/run"

ok "Installation complete!"
echo
say "Starting clock node now..."
sv-enable btcpc-clock 2>/dev/null || true
sv up btcpc-clock 2>/dev/null || nohup "$INSTALL_DIR/start.sh" > "$INSTALL_DIR/clock.log" 2>&1 &

sleep 2

ok "Clock node running for: $BTCPC_ACCOUNT"
echo
say "To check status:  sv status btcpc-clock"
say "To view logs:     tail -f $INSTALL_DIR/clock.log"
say "To stop:          sv down btcpc-clock"
say "To uninstall:     rm -rf $INSTALL_DIR && rm -rf $SVC_DIR"
echo
say "Important: keep Termux running in the background. Disable battery"
say "optimization for Termux in Android settings to prevent it from being killed."

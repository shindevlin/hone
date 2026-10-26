#!/data/data/com.termux/files/usr/bin/bash
# HONE Android Clock Node Installer
# For Termux on Android: install Termux from F-Droid first
# Usage: curl https://honemesh.net/android.sh | bash

set -e

ORANGE='\033[38;5;208m'
GREEN='\033[0;32m'
RESET='\033[0m'

say() { printf "${ORANGE}[hone]${RESET} %s\n" "$1"; }
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

INSTALL_DIR="$HOME/.hone-clock"
mkdir -p "$INSTALL_DIR"

say "Downloading hone-clock-lite..."
curl -fsSL https://honemesh.net/clock-lite.js -o "$INSTALL_DIR/clock.js"

# Prompt for account — read from /dev/tty so it works under curl | bash
echo
if [ -n "${HONE_ACCOUNT:-}" ]; then
  : # already set via env var
elif [ -r /dev/tty ]; then
  printf "${ORANGE}[hone]${RESET} Your HONE username: " > /dev/tty
  read -r HONE_ACCOUNT < /dev/tty
else
  echo "ERROR: No TTY available. Run with HONE_ACCOUNT=yourname before piping:"
  echo "  curl https://honemesh.net/android.sh | HONE_ACCOUNT=josh bash"
  echo "Or download first then run:"
  echo "  curl -o install.sh https://honemesh.net/android.sh && bash install.sh"
  exit 1
fi

if [[ ! "$HONE_ACCOUNT" =~ ^[a-z0-9][a-z0-9-]{2,19}$ ]]; then
  echo "Invalid username '$HONE_ACCOUNT' (3-20 chars, lowercase/numbers/hyphens)."
  exit 1
fi

# Save config
cat > "$INSTALL_DIR/config" <<EOF
HONE_CLOCK_ACCOUNT=$HONE_ACCOUNT
EOF

# Create launcher script
cat > "$INSTALL_DIR/start.sh" <<'EOF'
#!/data/data/com.termux/files/usr/bin/bash
source "$HOME/.hone-clock/config"
export HONE_CLOCK_ACCOUNT
cd "$HOME/.hone-clock"
exec node clock.js
EOF
chmod +x "$INSTALL_DIR/start.sh"

# Set up auto-start via termux-services (runit)
SVC_DIR="$PREFIX/var/service/hone-clock"
mkdir -p "$SVC_DIR/log"

cat > "$SVC_DIR/run" <<EOF
#!$PREFIX/bin/sh
exec 2>&1
source $INSTALL_DIR/config
export HONE_CLOCK_ACCOUNT
cd $INSTALL_DIR
exec node clock.js
EOF
chmod +x "$SVC_DIR/run"

cat > "$SVC_DIR/log/run" <<EOF
#!$PREFIX/bin/sh
exec logger -t hone-clock
EOF
chmod +x "$SVC_DIR/log/run"

ok "Installation complete!"
echo
say "Starting clock node now..."
sv-enable hone-clock 2>/dev/null || true
sv up hone-clock 2>/dev/null || nohup "$INSTALL_DIR/start.sh" > "$INSTALL_DIR/clock.log" 2>&1 &

sleep 2

ok "Clock node running for: $HONE_ACCOUNT"
echo
say "To check status:  sv status hone-clock"
say "To view logs:     tail -f $INSTALL_DIR/clock.log"
say "To stop:          sv down hone-clock"
say "To uninstall:     rm -rf $INSTALL_DIR && rm -rf $SVC_DIR"
echo
say "Important: keep Termux running in the background. Disable battery"
say "optimization for Termux in Android settings to prevent it from being killed."

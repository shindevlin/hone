#!/bin/sh
# HONE Birds camera listener installer: Linux (x86_64, Raspberry Pi arm64) and macOS.
#   curl -fsSL https://honemesh.net/app/listener/install.sh | sh
#   curl -fsSL https://honemesh.net/app/listener/install.sh | HONE_REF=invitecode sh
# Trust: version.json must carry a valid ed25519 signature (version.json.sig) by the pinned
# HONE listener release key, and every download must match its sha256 in that signed
# manifest, before anything is installed. HONE_VERIFIED_MANIFEST may point to a version.json
# that a trusted caller (the HONE MCP) already verified the same way.
# HONE_NONINTERACTIVE=1: never prompt (stdin closed); sign-in and camera choice happen in the
# local setup page in the browser.
set -eu
BASE="https://honemesh.net/app/listener"
PUBKEY_B64="MCowBQYDK2VwAyEAJ+9UWIZ0wzkeZgBW2HO4kTbnUSW1pDPeB0cZG6qUoQ0="
REF=$(printf '%s' "${HONE_REF:-}" | tr 'A-Z' 'a-z' | tr -cd 'a-z0-9._-' | cut -c1-32)
NONINT="${HONE_NONINTERACTIVE:-0}"

say() { printf '%s\n' "$*"; }
die() { printf 'HONE listener: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "this needs '$1' (please install it and run again)"; }
need curl
OS=$(uname -s); ARCH=$(uname -m)
case "$OS-$ARCH" in
  Linux-x86_64|Linux-amd64) PLAT=linux-x86_64; DEBARCH=amd64 ;;
  Linux-aarch64|Linux-arm64) PLAT=linux-aarch64; DEBARCH=arm64 ;;
  Darwin-*) PLAT=macos-universal ;;
  *) die "this computer ($OS $ARCH) isn't supported yet" ;;
esac

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT INT TERM
sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

if [ -n "${HONE_VERIFIED_MANIFEST:-}" ] && [ -f "$HONE_VERIFIED_MANIFEST" ]; then
  cp "$HONE_VERIFIED_MANIFEST" "$TMP/version.json"
  say "Using the release manifest that was already checked against the HONE release key."
else
  need openssl
  openssl version | grep -q '^OpenSSL [3-9]' || die "this needs OpenSSL 3 or newer to check the download's signature"
  curl -fsSL -o "$TMP/version.json" "$BASE/version.json" || die "couldn't reach honemesh.net; check your internet and try again"
  curl -fsSL -o "$TMP/version.json.sig" "$BASE/version.json.sig" || die "the release isn't signed yet, so nothing was installed"
  if tr -d ' \r\n' < "$TMP/version.json.sig" | grep -Eq '^[0-9a-fA-F]{128}$'; then
    tr -d ' \r\n' < "$TMP/version.json.sig" | sed 's/../& /g' | tr ' ' '\n' | grep . | while read -r h; do printf "\\$(printf '%03o' "0x$h")"; done > "$TMP/sig.bin"
  else
    openssl base64 -d -A -in "$TMP/version.json.sig" -out "$TMP/sig.bin"
  fi
  { echo "-----BEGIN PUBLIC KEY-----"; echo "$PUBKEY_B64"; echo "-----END PUBLIC KEY-----"; } > "$TMP/pub.pem"
  openssl pkeyutl -verify -pubin -inkey "$TMP/pub.pem" -rawin -in "$TMP/version.json" -sigfile "$TMP/sig.bin" >/dev/null 2>&1 \
    || die "version.json's signature didn't match the HONE release key; nothing was installed"
  say "Checked: the release is signed by HONE."
fi
FLAT=$(tr -d ' \r\n\t' < "$TMP/version.json")
printf '%s' "$FLAT" | grep -q '"name":"hone-listener"' || die "the release manifest isn't for the HONE listener"
VER=$(printf '%s' "$FLAT" | grep -o '"version":"[0-9][0-9.]*"' | head -1 | cut -d'"' -f4)
[ -n "$VER" ] || die "the release manifest has no version"
want_sha() { printf '%s' "$FLAT" | grep -o "\"$(printf '%s' "$1" | sed 's/\./\\./g')\":{[^}]*}" | grep -o '"sha256":"[0-9a-f]\{64\}"' | head -1 | cut -d'"' -f4; }
fetch() { # name -> $TMP/name, checked against the signed sha256
  W=$(want_sha "$1"); [ -n "$W" ] || die "the signed manifest has no checksum for $1"
  curl -fsSL -o "$TMP/$1" "$BASE/$1" || die "download of $1 failed"
  [ "$(sha256 "$TMP/$1")" = "$W" ] || die "$1 didn't match its signed checksum; nothing was installed"
}

say "Installing the HONE listener $VER…"
if [ "$OS" = Linux ]; then
  DEB="hone-listener_${VER}_${DEBARCH}.deb"
  SUDO=""; CANROOT=0
  if [ "$(id -u)" -eq 0 ]; then CANROOT=1
  elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then SUDO="sudo -n"; CANROOT=1
  elif command -v sudo >/dev/null 2>&1 && [ "$NONINT" != 1 ] && [ -r /dev/tty ]; then SUDO="sudo"; CANROOT=1; fi
  if [ $CANROOT = 1 ] && command -v apt-get >/dev/null 2>&1; then
    fetch "$DEB"; say "Checked: $DEB matches the signed checksum."
    chmod 0755 "$TMP"; chmod 0644 "$TMP/$DEB"
    $SUDO env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$TMP/$DEB" </dev/null
    if [ -n "$REF" ]; then printf '%s\n' "$REF" | $SUDO tee /var/lib/hone-listener/ref >/dev/null; $SUDO chown hone-listener:hone-listener /var/lib/hone-listener/ref; fi
    U="${SUDO_USER:-${USER:-}}"; if [ -n "$U" ] && [ "$U" != root ]; then $SUDO usermod -aG hone-listener "$U" || true; fi
    HEADLESS=0; [ -z "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] && HEADLESS=1
    # No screen here: let the setup page open from a phone on the same Wi-Fi (still needs its private link).
    [ $HEADLESS = 1 ] && $SUDO sed -i 's/^#HONE_LISTENER_UI=lan/HONE_LISTENER_UI=lan/' /etc/default/hone-listener
    $SUDO systemctl restart hone-listener.service
    i=0; URL=""
    while [ $i -lt 20 ] && [ -z "$URL" ]; do sleep 1; i=$((i+1)); URL=$($SUDO hone-listener url $([ $HEADLESS = 1 ] && echo --lan) 2>/dev/null || true); done
    say ""; say "The HONE listener is installed and starts by itself whenever this computer starts."
  else
    # No admin rights here: install for this user only (systemd user service).
    command -v ffmpeg >/dev/null 2>&1 || die "this needs ffmpeg; ask the computer's owner to run: sudo apt-get install ffmpeg"
    command -v dpkg-deb >/dev/null 2>&1 || die "this needs dpkg-deb (Debian, Ubuntu or Raspberry Pi OS)"
    fetch "$DEB"; say "Checked: $DEB matches the signed checksum."
    APP="$HOME/.local/share/hone-listener/app"; STATE="$HOME/.local/share/hone-listener"
    systemctl --user stop hone-listener.service 2>/dev/null || true
    dpkg-deb -x "$TMP/$DEB" "$TMP/x"; mkdir -p "$APP"; cp -f "$TMP"/x/usr/lib/hone-listener/* "$APP/"
    [ -n "$REF" ] && printf '%s\n' "$REF" > "$STATE/ref"
    mkdir -p "$HOME/.config/systemd/user"
    cat > "$HOME/.config/systemd/user/hone-listener.service" <<UNIT
[Unit]
Description=HONE Birds camera listener
After=network-online.target
[Service]
ExecStart=$APP/hone-listener run
Environment=HONE_LISTENER_SUPERVISED=1
Restart=always
RestartSec=10
[Install]
WantedBy=default.target
UNIT
    systemctl --user daemon-reload; systemctl --user enable --now hone-listener.service
    loginctl enable-linger "$(id -un)" 2>/dev/null || say "(It starts when you log in. To start with the computer, ask an admin to run: sudo loginctl enable-linger $(id -un))"
    i=0; URL=""; while [ $i -lt 20 ] && [ -z "$URL" ]; do sleep 1; i=$((i+1)); URL=$("$APP/hone-listener" url 2>/dev/null || true); done
    say ""; say "The HONE listener is installed for you and runs in the background."
  fi
  if [ -n "$URL" ]; then
    say "Finish setup on this page (keep the link private, it opens your listener):"; say "  $URL"
    if [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] && command -v xdg-open >/dev/null 2>&1; then xdg-open "$URL" >/dev/null 2>&1 || true; fi
  else
    say "Finish setup: run  hone-listener open  (or  sudo hone-listener url ) and open the link."
  fi
  exit 0
fi

# macOS: per-user install + launchd agent (starts at login, menu-bar icon).
TGZ="hone-listener-${VER}-macos-universal.tar.gz"
fetch "$TGZ"; say "Checked: $TGZ matches the signed checksum."
STATE="$HOME/Library/Application Support/HONE Listener"; APP="$STATE/app"
launchctl bootout "gui/$(id -u)/net.honemesh.listener" 2>/dev/null || true
mkdir -p "$APP"; tar -xzf "$TMP/$TGZ" -C "$APP"
xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true
[ -n "$REF" ] && printf '%s\n' "$REF" > "$STATE/ref"
PL="$HOME/Library/LaunchAgents/net.honemesh.listener.plist"; mkdir -p "$HOME/Library/LaunchAgents"
cat > "$PL" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>net.honemesh.listener</string>
  <key>ProgramArguments</key><array><string>$APP/hone-listener</string><string>tray</string></array>
  <key>EnvironmentVariables</key><dict><key>HONE_LISTENER_SUPERVISED</key><string>1</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardErrorPath</key><string>$STATE/listener.log</string>
</dict></plist>
PLIST
launchctl bootstrap "gui/$(id -u)" "$PL"
say "The HONE listener is installed and starts when you log in."
say "Its setup page opens in your browser; it's also in the menu bar."

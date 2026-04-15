#!/usr/bin/env bash
set -euo pipefail

# ── Integrity check ────────────────────────────────────────────────────────────
# If this script was fetched from btcpc.net (or anywhere else), verify it
# matches the canonical copy on GitHub before executing anything privileged.
# Skip with BTCPC_SKIP_VERIFY=1 for offline / air-gapped installs.
GITHUB_RAW="https://raw.githubusercontent.com/shindevlin/btcpc/main/scripts/install-user.sh"

if [[ "${BTCPC_SKIP_VERIFY:-0}" != "1" ]] && command -v sha256sum >/dev/null 2>&1; then
  echo "[btcpc] Verifying script integrity against GitHub..."
  _SELF="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
  # When piped via `curl | bash`, BASH_SOURCE[0] is /dev/stdin — fall back to a
  # temp download for comparison in that case.
  if [[ "$_SELF" == *"/dev/stdin"* || "$_SELF" == "/dev/stdin" || ! -f "$_SELF" ]]; then
    _TMP="$(mktemp)"
    if curl -fsSL --max-time 15 "$GITHUB_RAW" -o "$_TMP" 2>/dev/null; then
      _GITHUB_HASH="$(sha256sum "$_TMP" | awk '{print $1}')"
      # Re-fetch self from the same source we were called from and compare
      _SELF_TMP="$(mktemp)"
      _SELF_URL="${BTCPC_INSTALL_URL:-https://btcpc.net/install.sh}"
      if curl -fsSL --max-time 15 "$_SELF_URL" -o "$_SELF_TMP" 2>/dev/null; then
        _SELF_HASH="$(sha256sum "$_SELF_TMP" | awk '{print $1}')"
        if [[ "$_GITHUB_HASH" != "$_SELF_HASH" ]]; then
          echo ""
          echo "ERROR: Script integrity check failed."
          echo "  btcpc.net hash : $_SELF_HASH"
          echo "  GitHub hash    : $_GITHUB_HASH"
          echo ""
          echo "The script served by btcpc.net does not match GitHub."
          echo "This could indicate a compromised server or a deployment in progress."
          echo "To skip this check: BTCPC_SKIP_VERIFY=1 bash <(curl -fsSL $GITHUB_RAW)"
          echo "Or install directly from GitHub: bash <(curl -fsSL $GITHUB_RAW)"
          rm -f "$_TMP" "$_SELF_TMP"
          exit 1
        fi
        echo "[btcpc] Integrity OK (matches GitHub sha256: ${_GITHUB_HASH:0:16}...)"
      else
        echo "[btcpc] Could not re-fetch self for comparison — skipping verify"
      fi
      rm -f "$_TMP" "$_SELF_TMP" 2>/dev/null || true
    else
      echo "[btcpc] Could not reach GitHub for verification — skipping (set BTCPC_SKIP_VERIFY=1 to suppress)"
    fi
  fi
fi
# ── End integrity check ────────────────────────────────────────────────────────

USERNAME="${1:-}"
REPO_URL="${BTCPC_REPO_URL:-https://github.com/shindevlin/btcpc.git}"
INSTALL_DIR="${BTCPC_INSTALL_DIR:-$HOME/btcpc}"

if [[ -z "$USERNAME" ]]; then
  echo "Usage: bash install.sh <username>"
  echo "       curl -fsSL https://btcpc.net/install.sh | bash -s -- <username>"
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

echo "[btcpc] Installing BTCPC for user: $USERNAME"

# ── Dependencies ───────────────────────────────────────────────────────────────
sudo apt-get update -qq
sudo apt-get install -y curl git ca-certificates openssl

# Node.js 20+
if ! command -v node >/dev/null 2>&1 || [[ "$(node -v 2>/dev/null | sed 's/^v//;s/\..*//')" -lt 20 ]]; then
  echo "[btcpc] Installing Node.js 20..."
  curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
  sudo apt-get install -y nodejs
fi

# Ollama (optional — needed for mining)
if [[ "${BTCPC_SKIP_OLLAMA:-0}" != "1" ]]; then
  if ! command -v ollama >/dev/null 2>&1; then
    echo "[btcpc] Installing Ollama..."
    curl -fsSL https://ollama.ai/install.sh | sh
  fi
fi

# ── Clone / update repo ────────────────────────────────────────────────────────
if [[ ! -d "$INSTALL_DIR/.git" ]]; then
  git clone "$REPO_URL" "$INSTALL_DIR"
else
  git -C "$INSTALL_DIR" pull --ff-only
fi

cd "$INSTALL_DIR"
npm install --omit=dev --silent

# ── Generate secrets ───────────────────────────────────────────────────────────
JWT_SECRET_VALUE="$(openssl rand -hex 32)"

# Only write .env if it doesn't already exist (preserve existing config on updates)
if [[ ! -f .env ]]; then
  cat > .env <<EOF
PORT=3000
NODE_ENV=production
JWT_SECRET=$JWT_SECRET_VALUE
JWT_EXPIRES_IN=7d
BTCPC_MONGO_MODE=disabled
RATE_LIMIT_WINDOW_MS=900000
RATE_LIMIT_MAX_REQUESTS=100
EOF
  echo "[btcpc] Generated .env with fresh JWT secret"
else
  echo "[btcpc] .env already exists — skipping (delete it to reset config)"
fi

# ── Create account ─────────────────────────────────────────────────────────────
BTCPC_MINER="$USERNAME" \
BTCPC_CLOCK_ACCOUNT="$USERNAME" \
node bin/btcpc-setup --yes --username "$USERNAME" --skip-ollama

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo " BTCPC installed successfully"
echo " Username : $USERNAME"
echo " API      : http://localhost:3000"
echo " Explorer : http://localhost:4242"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "Start the node:  cd $INSTALL_DIR && node bin/btcpc-all"
echo "Mine tokens:     cd $INSTALL_DIR && BTCPC_MINER=$USERNAME node bin/btcpc-all"
echo ""
echo "To verify this script yourself before running:"
echo "  curl -fsSL https://btcpc.net/install.sh -o install.sh"
echo "  cat install.sh        # inspect it"
echo "  bash install.sh $USERNAME"

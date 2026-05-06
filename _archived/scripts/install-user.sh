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

# ── Capability wizard ──────────────────────────────────────────────────────────
# Only run interactively — skip if piped (curl | bash) or BTCPC_NONINTERACTIVE=1
if [[ "${BTCPC_NONINTERACTIVE:-0}" != "1" ]] && [[ -t 0 ]]; then
  echo ""
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo " BTCPC Agent Capabilities"
  echo " You earn BTCPC for every job your machine processes."
  echo " Choose what you want to allow. You can change these later in .env"
  echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
  echo ""

  _ask_yn() {
    local prompt="$1" default="$2" answer
    while true; do
      printf "%s [%s]: " "$prompt" "$default"
      read -r answer
      answer="${answer:-$default}"
      case "${answer,,}" in
        y|yes) echo "true"; return ;;
        n|no)  echo "false"; return ;;
        *)     echo "  Please answer y or n." >&2 ;;
      esac
    done
  }

  _ask_choice() {
    local prompt="$1" default="$2" options="$3" answer
    while true; do
      printf "%s (%s) [%s]: " "$prompt" "$options" "$default"
      read -r answer
      answer="${answer:-$default}"
      if [[ "|${options//\//|}|" == *"|${answer}|"* ]]; then
        echo "$answer"; return
      else
        echo "  Please choose one of: $options" >&2
      fi
    done
  }

  echo "── Inference jobs (text generation, AI chat) ──────────────────────────────"
  echo "   Standard inference is always enabled — it's the core of BTCPC mining."
  echo ""

  echo "── Code execution (sandboxed JavaScript / Python / Bash) ─────────────────"
  echo "   Agents can ask your machine to run code in an isolated subprocess."
  echo "   The sandbox has no network access and a 30-second timeout."
  echo ""
  _CODE_EXEC=$(_ask_yn "   Allow code execution jobs?" "y")

  echo ""
  echo "── Browser / computer-use jobs (headless web automation) ─────────────────"
  echo "   Agents can control a headless browser on your machine to complete"
  echo "   web tasks. The browser is NEVER shown on your screen — buyer sessions"
  echo "   are private. Screenshots are encrypted and sent to the buyer only."
  echo ""
  _BROWSER=$(_ask_yn "   Allow browser agent jobs?" "y")

  if [[ "$_BROWSER" == "true" ]]; then
    echo ""
    echo "   Content filter — hard blocks always active: SSRF/private IPs, CSAM."
    echo "   All categories below default to allowed. Answer 'y' to block on your machine."
    echo ""
    echo "   Tor / .onion hidden services: Used by SecureDrop, privacy-focused news"
    echo "   outlets, whistleblower platforms, and censorship-evading services."
    echo "   BTCPC does not block these by default — censorship resistance is core"
    echo "   to the protocol."
    _BLOCK_ONION=$(_ask_yn "   Block Tor/.onion hidden services?" "n")
    echo ""
    echo "   Adult content: Legal adult websites (pornography, adult entertainment)."
    _BLOCK_ADULT=$(_ask_yn "   Block adult content sites?" "n")
    echo ""
    echo "   Gambling: Online casinos, sports betting, poker sites."
    _BLOCK_GAMBLING=$(_ask_yn "   Block gambling sites?" "n")
    echo ""
    echo "   Drug marketplaces: Sites selling illegal controlled substances."
    _BLOCK_DRUGS=$(_ask_yn "   Block drug marketplace sites?" "n")
    echo ""
    echo "   Weapons: Sites selling illegal firearms or weapons."
    _BLOCK_WEAPONS=$(_ask_yn "   Block illegal weapons sites?" "n")
    echo ""
    printf "   Restrict to specific domains only? (leave blank to allow all): "
    read -r _ALLOWED_DOMAINS
    _ALLOWED_DOMAINS="${_ALLOWED_DOMAINS// /}"  # strip spaces
  fi

  echo ""
  echo "── Fine-tuning jobs (LoRA model training) ─────────────────────────────────"
  echo "   Buyers pay you to fine-tune AI models on their datasets."
  echo "   Uses significant GPU and disk space. Jobs can run for hours."
  echo ""
  _FINETUNE=$(_ask_yn "   Allow fine-tuning jobs?" "n")

  echo ""
  echo "── Inference tier ──────────────────────────────────────────────────────────"
  echo "   standard = general models (qwen3:4b, llama3, etc.)"
  echo "   reasoning = extended thinking models (qwq:32b, deepseek-r1)"
  echo "   fast      = small/fast models (qwen3:0.6b, phi3-mini)"
  echo ""
  _TIER=$(_ask_choice "   Preferred inference tier" "standard" "standard/reasoning/fast")

  # Write capability settings to .env
  {
    echo ""
    echo "# BTCPC Agent Capabilities (set during install)"
    echo "BTCPC_CODE_EXEC_ENABLED=${_CODE_EXEC}"
    echo "BTCPC_BROWSER_ENABLED=${_BROWSER}"
    if [[ "$_BROWSER" == "true" ]]; then
      echo "BTCPC_BROWSER_BLOCK_ONION=${_BLOCK_ONION}"
      echo "BTCPC_BROWSER_BLOCK_ADULT=${_BLOCK_ADULT}"
      echo "BTCPC_BROWSER_BLOCK_GAMBLING=${_BLOCK_GAMBLING}"
      echo "BTCPC_BROWSER_BLOCK_DRUGS=${_BLOCK_DRUGS}"
      echo "BTCPC_BROWSER_BLOCK_WEAPONS=${_BLOCK_WEAPONS}"
      if [[ -n "$_ALLOWED_DOMAINS" ]]; then
        echo "BTCPC_BROWSER_ALLOWED_DOMAINS=${_ALLOWED_DOMAINS}"
      fi
    fi
    echo "BTCPC_FINETUNE_ENABLED=${_FINETUNE}"
    echo "BTCPC_DEFAULT_TIER=${_TIER}"
  } >> .env

  # Install Playwright if browser jobs enabled
  if [[ "$_BROWSER" == "true" ]]; then
    echo ""
    echo "[btcpc] Installing Playwright (headless browser for agent jobs)..."
    npm install playwright --save-dev --silent 2>/dev/null || true
    npx playwright install chromium --with-deps 2>/dev/null || \
      echo "[btcpc] Playwright chromium install failed — browser jobs will be skipped at runtime"
  fi

  # Install fine-tuning dependencies if enabled
  if [[ "$_FINETUNE" == "true" ]]; then
    echo "[btcpc] Fine-tuning support noted — ensure Ollama has sufficient VRAM for your chosen models."
  fi

  echo ""
  echo "[btcpc] Capability settings written to .env"
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

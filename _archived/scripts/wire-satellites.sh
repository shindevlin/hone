#!/usr/bin/env bash
# wire-satellites.sh — sync all hone satellite repos to the correct API endpoint.
# Run this whenever the canonical API URL changes.
#
# Usage:
#   ./scripts/wire-satellites.sh                  # set all to https://hone.net
#   HONE_API_URL=https://staging.hone.net ./scripts/wire-satellites.sh

set -euo pipefail

CANONICAL="${HONE_API_URL:-https://hone.net}"
REPOS_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CHANGED=0

patch_env() {
  local env_file="$1"
  if [[ ! -f "$env_file" ]]; then return; fi
  if grep -q "HONE_API_URL=" "$env_file"; then
    local current
    current=$(grep "HONE_API_URL=" "$env_file" | head -1 | cut -d= -f2-)
    if [[ "$current" != "$CANONICAL" ]]; then
      sed -i "s|HONE_API_URL=.*|HONE_API_URL=${CANONICAL}|" "$env_file"
      echo "  [.env]  $env_file  →  $CANONICAL"
      CHANGED=$((CHANGED + 1))
    fi
  fi
}

patch_js_fallback() {
  local js_file="$1"
  local old_pattern="$2"
  if [[ ! -f "$js_file" ]]; then return; fi
  if grep -q "process.env.HONE_API_URL || '" "$js_file"; then
    local current
    current=$(grep "process.env.HONE_API_URL || '" "$js_file" | head -1 | sed "s/.*|| '//;s/'.*//")
    if [[ "$current" != "$CANONICAL" ]]; then
      sed -i "s|process.env.HONE_API_URL || '${old_pattern}'|process.env.HONE_API_URL || '${CANONICAL}'|g" "$js_file"
      echo "  [js]    $js_file  →  $CANONICAL"
      CHANGED=$((CHANGED + 1))
    fi
  fi
}

echo "=== hone satellite wiring sync ==="
echo "Target: $CANONICAL"
echo ""

# betchu_bot
if [[ -d "$REPOS_ROOT/betchu_bot" ]]; then
  echo "betchu_bot:"
  env_file="$REPOS_ROOT/betchu_bot/.env"
  if [[ -f "$env_file" ]]; then
    if grep -q "HONE_API_URL=" "$env_file"; then
      patch_env "$env_file"
    else
      # Not yet wired — insert it
      sed -i "s|# HONE Node|# HONE Node\nHONE_API_URL=${CANONICAL}|" "$env_file" 2>/dev/null || true
      echo "  [.env]  $env_file  →  $CANONICAL (inserted)"
      CHANGED=$((CHANGED + 1))
    fi
  fi
fi

# honebot
echo "honebot:"
patch_env      "$REPOS_ROOT/honebot/.env"
patch_js_fallback "$REPOS_ROOT/honebot/index.js" "https://hone.net"

# honewalletbot (alertbot)
echo "honewalletbot:"
patch_env      "$REPOS_ROOT/honewalletbot/.env"
patch_js_fallback "$REPOS_ROOT/honewalletbot/index.js" "https://hone.net"

# alertbot (standalone)
if [[ -d "$REPOS_ROOT/alertbot" ]]; then
  echo "alertbot:"
  patch_env      "$REPOS_ROOT/alertbot/.env"
  patch_js_fallback "$REPOS_ROOT/alertbot/index.js" "https://hone.net"
fi

# hone-desktop (Tauri v1 — Rust, uses HONE_API_URL env at runtime)
if [[ -d "$REPOS_ROOT/hone-desktop" ]]; then
  echo "hone-desktop:"
  env_rs="$REPOS_ROOT/hone-desktop/src-tauri/src/main.rs"
  if [[ -f "$env_rs" ]] && grep -q "https://hone.net" "$env_rs"; then
    old=$(grep -o '"https://[^"]*"' "$env_rs" | head -1 | tr -d '"')
    if [[ "$old" != "$CANONICAL" ]]; then
      sed -i "s|\"${old}\"|\"${CANONICAL}\"|g" "$env_rs"
      echo "  [rs]    $env_rs  →  $CANONICAL"
      CHANGED=$((CHANGED + 1))
    fi
  fi
fi

# bullship
if [[ -d "$REPOS_ROOT/bullship" ]]; then
  echo "bullship:"
  patch_env "$REPOS_ROOT/bullship/.envhone"
  patch_js_fallback "$REPOS_ROOT/bullship/queue_worker/index.js" "https://hone.net"
  patch_js_fallback "$REPOS_ROOT/bullship/oracle/index.js"       "https://hone.net"
  patch_js_fallback "$REPOS_ROOT/bullship/ai-headline/index.js"  "https://hone.net"
fi

# brutus11
if [[ -d "$REPOS_ROOT/brutus11" ]]; then
  echo "brutus11:"
  patch_env "$REPOS_ROOT/brutus11/.envhone"
fi

# BusWingSpread
if [[ -d "$REPOS_ROOT/BusWingSpread" ]]; then
  echo "BusWingSpread:"
  patch_env "$REPOS_ROOT/BusWingSpread/.envhone"
  py="$REPOS_ROOT/BusWingSpread/hone_client.py"
  if [[ -f "$py" ]]; then
    current=$(grep 'HONE_URL.*getenv' "$py" | head -1 | grep -o '"https://[^"]*"' | tr -d '"' || true)
    if [[ -n "$current" && "$current" != "$CANONICAL" ]]; then
      sed -i "s|\"${current}\"|\"${CANONICAL}\"|g" "$py"
      echo "  [py]    $py  →  $CANONICAL"
      CHANGED=$((CHANGED + 1))
    fi
  fi
fi

# counselflow
if [[ -d "$REPOS_ROOT/counselflow" ]]; then
  echo "counselflow:"
  patch_env "$REPOS_ROOT/counselflow/.envhone"
fi

echo ""
if [[ $CHANGED -eq 0 ]]; then
  echo "All satellite repos already wired to $CANONICAL. Nothing changed."
else
  echo "$CHANGED file(s) updated."
fi

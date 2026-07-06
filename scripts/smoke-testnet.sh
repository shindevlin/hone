#!/usr/bin/env bash
# smoke-testnet.sh — run the go-live smoke test suite against the public testnet
#
# Usage:
#   bash scripts/smoke-testnet.sh                    # targets https://honemesh.net
#   bash scripts/smoke-testnet.sh https://honemesh.net  # explicit URL
#   bash scripts/smoke-testnet.sh http://localhost:4242  # local node
#
# Optional env vars:
#   HONE_SMOKE_API_KEY  — API key for authenticated inference test

set -euo pipefail

TARGET="${1:-https://honemesh.net}"

echo "Running go-live smoke tests against: ${TARGET}"
echo ""

export HONE_SMOKE_URL="${TARGET}"
export HONE_SMOKE_SKIP=0

npm test -- --testPathPattern=smoke --forceExit

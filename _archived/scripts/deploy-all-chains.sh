#!/bin/bash
# Deploy wHONE on all supported chains from a single Base ETH funding
# Usage: bash scripts/deploy-all-chains.sh
#
# Prerequisites:
#   - Foundry (forge, cast) installed
#   - Solana CLI installed
#   - ETH funded on Base to shin's address
#   - Private keys in .env

set -e

export PATH="$HOME/.foundry/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(dirname "$SCRIPT_DIR")"
source "$REPO_DIR/.env"

# Shin's EVM private key (derived from mnemonic)
EVM_KEY="${HONE_SHIN_EVM_KEY:?Set HONE_SHIN_EVM_KEY in .env}"
EVM_ADDRESS="${HONE_SHIN_ADDRESS:-0xbde88f2b3a224b242704bd166804e0e12c75e830}"

# Signing authority for the wHONE contract (shin's EVM address verifies claim proofs)
SIGNING_AUTHORITY="$EVM_ADDRESS"

echo "============================================"
echo "  wHONE Deployment — All Chains"
echo "============================================"
echo ""

# ─── Check balances ───────────────────────────────────────────────

echo "[deploy] Checking balances..."

BASE_BAL=$(cast balance $EVM_ADDRESS --rpc-url https://mainnet.base.org 2>/dev/null || echo "0")
echo "  Base:     $(cast from-wei $BASE_BAL 2>/dev/null || echo "$BASE_BAL wei") ETH"

ARB_BAL=$(cast balance $EVM_ADDRESS --rpc-url https://arb1.arbitrum.io/rpc 2>/dev/null || echo "0")
echo "  Arbitrum: $(cast from-wei $ARB_BAL 2>/dev/null || echo "$ARB_BAL wei") ETH"

OP_BAL=$(cast balance $EVM_ADDRESS --rpc-url https://mainnet.optimism.io 2>/dev/null || echo "0")
echo "  Optimism: $(cast from-wei $OP_BAL 2>/dev/null || echo "$OP_BAL wei") ETH"

ETH_BAL=$(cast balance $EVM_ADDRESS --rpc-url https://rpc.flashbots.net 2>/dev/null || echo "0")
echo "  Ethereum: $(cast from-wei $ETH_BAL 2>/dev/null || echo "$ETH_BAL wei") ETH"

echo ""

# ─── Deploy on Base ───────────────────────────────────────────────

deploy_evm() {
  local CHAIN_NAME=$1
  local RPC_URL=$2
  local CHAIN_ID=$3

  echo "[deploy] Deploying wHONE on $CHAIN_NAME (chain $CHAIN_ID)..."

  DEPLOY_OUTPUT=$(cd "$REPO_DIR" && forge create contracts/wHONE.sol:wHONE \
    --rpc-url "$RPC_URL" \
    --private-key "$EVM_KEY" \
    --constructor-args "$SIGNING_AUTHORITY" \
    --json 2>&1)

  CONTRACT=$(echo "$DEPLOY_OUTPUT" | node -e "var d='';process.stdin.on('data',c=>d+=c);process.stdin.on('end',()=>{try{console.log(JSON.parse(d).deployedTo)}catch(e){console.log('FAILED: '+d.slice(0,200))}})")

  if [[ "$CONTRACT" == 0x* ]]; then
    echo "  [OK] wHONE deployed: $CONTRACT"
    echo "  Set in .env: WHONE_${CHAIN_NAME}_CONTRACT=$CONTRACT"
    echo "WHONE_${CHAIN_NAME}_CONTRACT=$CONTRACT" >> "$REPO_DIR/.env"
  else
    echo "  [FAIL] $CONTRACT"
  fi
  echo ""
}

if [ "$BASE_BAL" != "0" ]; then
  deploy_evm "BASE" "https://mainnet.base.org" 8453
fi

# ─── Deploy on other EVM chains (if funded) ───────────────────────

if [ "$ARB_BAL" != "0" ]; then
  deploy_evm "ARB" "https://arb1.arbitrum.io/rpc" 42161
fi

if [ "$OP_BAL" != "0" ]; then
  deploy_evm "OP" "https://mainnet.optimism.io" 10
fi

if [ "$ETH_BAL" != "0" ]; then
  deploy_evm "ETH" "https://rpc.flashbots.net" 1
fi

# ─── Solana (if SOL balance available) ────────────────────────────

echo "[deploy] Checking Solana..."
SOL_ADDR="NGLFShxm7JeBLKqm3yUTpjwcKsvcyFKU52jNLH8SHNC"
SOL_BAL=$(solana balance $SOL_ADDR --url mainnet-beta 2>/dev/null || echo "0 SOL")
echo "  Solana balance: $SOL_BAL"

if [[ "$SOL_BAL" != "0 SOL" && "$SOL_BAL" != "0" ]]; then
  echo "[deploy] Creating SPL token on Solana..."
  # Create mint, create metadata, etc.
  # spl-token create-token --decimals 10
  echo "  [TODO] Solana SPL token deployment — needs keypair import"
fi

# ─── Hive (free) ─────────────────────────────────────────────────

echo "[deploy] Hive: No deployment needed — claims use custom_json (free)"
echo "  Hive account: shindevlin"
echo "  Already built in src/claims/hiveClaimManager.js"
echo ""

# ─── Summary ──────────────────────────────────────────────────────

echo "============================================"
echo "  Deployment Complete"
echo "============================================"
echo ""
echo "Check .env for contract addresses."
echo "Restart the miner to start submitting claims."
echo ""

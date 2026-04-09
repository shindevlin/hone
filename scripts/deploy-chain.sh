#!/bin/bash
# Deploy wBTCPC + Sale + Bridge on a specific EVM chain
# Usage: bash scripts/deploy-chain.sh <chain>
# Chains: base, arbitrum, optimism, ethereum

export PATH="$HOME/.foundry/bin:$PATH"
cd "$(dirname "$0")/.."

CHAIN=$1
KEY="d9409e6735ea61fb5408e66fe95fb3cc078bd5033915a4d5a3714dfd77a8f6bb"
SHIN="0xBDe88F2B3a224B242704bD166804E0E12c75e830"

# Chain configs
case $CHAIN in
  base)      RPC="https://mainnet.base.org"; USDC="0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"; DAI="0x50c5725949A6F0c72E6C4a641F24049A917DB0Cb"; USDBC="0xd9aAEc86B65D86f6A7B5B1b0c42FFA531710b6CA" ;;
  arbitrum)  RPC="https://arb1.arbitrum.io/rpc"; USDC="0xaf88d065e77c8cC2239327C5EDb3A432268e5831"; USDT="0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9"; DAI="0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1" ;;
  optimism)  RPC="https://mainnet.optimism.io"; USDC="0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85"; USDT="0x94b008aA00579c1307B0EF2c499aD98a8ce58e58"; DAI="0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1" ;;
  ethereum)  RPC="https://rpc.flashbots.net"; USDC="0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"; USDT="0xdAC17F958D2ee523a2206206994597C13D831ec7"; DAI="0x6B175474E89094C44Da98b954EedeAC495271d0F" ;;
  *) echo "Usage: $0 <base|arbitrum|optimism|ethereum>"; exit 1 ;;
esac

echo "============================================"
echo "  Deploying wBTCPC suite on $CHAIN"
echo "============================================"

# 1. Deploy wBTCPC token
echo "[1/3] Deploying wBTCPC token..."
TOKEN_OUT=$(forge create "contracts/wBTCPC.sol:wBTCPC" --broadcast \
  --rpc-url "$RPC" --private-key "$KEY" \
  --constructor-args "$SHIN" 2>&1)
TOKEN=$(echo "$TOKEN_OUT" | grep "Deployed to:" | awk '{print $3}')
echo "  wBTCPC: $TOKEN"

if [ -z "$TOKEN" ]; then echo "  FAILED - check gas balance"; exit 1; fi

# 2. Deploy Sale contract
echo "[2/3] Deploying wBTCPCSale..."
SALE_OUT=$(forge create "contracts/wBTCPCSale.sol:wBTCPCSale" --broadcast \
  --rpc-url "$RPC" --private-key "$KEY" \
  --constructor-args "$TOKEN" "$SHIN" "100000" 2>&1)
SALE=$(echo "$SALE_OUT" | grep "Deployed to:" | awk '{print $3}')
echo "  Sale: $SALE"

# Add stablecoins
if [ -n "$USDC" ]; then
  echo "  Adding USDC..."
  cast send $SALE "addStablecoin(address,uint8)" "$USDC" 6 --rpc-url "$RPC" --private-key "$KEY" 2>&1 | grep "status"
fi
if [ -n "$USDT" ]; then
  echo "  Adding USDT..."
  cast send $SALE "addStablecoin(address,uint8)" "$USDT" 6 --rpc-url "$RPC" --private-key "$KEY" 2>&1 | grep "status"
fi
if [ -n "$DAI" ]; then
  echo "  Adding DAI..."
  cast send $SALE "addStablecoin(address,uint8)" "$DAI" 18 --rpc-url "$RPC" --private-key "$KEY" 2>&1 | grep "status"
fi
if [ -n "$USDBC" ]; then
  echo "  Adding USDbC..."
  cast send $SALE "addStablecoin(address,uint8)" "$USDBC" 6 --rpc-url "$RPC" --private-key "$KEY" 2>&1 | grep "status"
fi

# 3. Deploy Bridge
echo "[3/3] Deploying wBTCPCBridge..."
BRIDGE_OUT=$(forge create "contracts/wBTCPCBridge.sol:wBTCPCBridge" --broadcast \
  --rpc-url "$RPC" --private-key "$KEY" \
  --constructor-args "$TOKEN" "$SHIN" 2>&1)
BRIDGE=$(echo "$BRIDGE_OUT" | grep "Deployed to:" | awk '{print $3}')
echo "  Bridge: $BRIDGE"

# Bind shin's account
echo "  Binding shindevlin..."
cast send $BRIDGE "bindAccount(string,address)" "shindevlin" "$SHIN" --rpc-url "$RPC" --private-key "$KEY" 2>&1 | grep "status"

echo ""
echo "============================================"
echo "  $CHAIN deployment complete"
echo "============================================"
echo "  wBTCPC:  $TOKEN"
echo "  Sale:    $SALE"
echo "  Bridge:  $BRIDGE"
echo "============================================"

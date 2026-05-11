#!/usr/bin/env bash
# Verify BridgeLock and BridgeReserve on Basescan after deployment.
# Usage: BRIDGE_LOCK=0x... BRIDGE_RESERVE=0x... SAFE=0x... bash contracts/deploy/verify_bridge.sh
set -euo pipefail

: "${BRIDGE_LOCK:?Set BRIDGE_LOCK}"
: "${BRIDGE_RESERVE:?Set BRIDGE_RESERVE}"
: "${SAFE:?Set SAFE}"
: "${BASESCAN_API_KEY:?Set BASESCAN_API_KEY}"

echo "Verifying BridgeLock at $BRIDGE_LOCK ..."
forge verify-contract \
  --chain-id 8453 \
  --etherscan-api-key "$BASESCAN_API_KEY" \
  --constructor-args $(cast abi-encode "constructor(address)" "$SAFE") \
  "$BRIDGE_LOCK" \
  contracts/BridgeLock.sol:BridgeLock

echo "Verifying BridgeReserve at $BRIDGE_RESERVE ..."
RELAYER="${RELAYER_ADDRESS:-$SAFE}"
forge verify-contract \
  --chain-id 8453 \
  --etherscan-api-key "$BASESCAN_API_KEY" \
  --constructor-args $(cast abi-encode "constructor(address,address)" "$SAFE" "$RELAYER") \
  "$BRIDGE_RESERVE" \
  contracts/BridgeReserve.sol:BridgeReserve

echo "Done. Check https://basescan.org/address/$BRIDGE_LOCK and $BRIDGE_RESERVE"

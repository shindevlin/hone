#!/bin/bash
# EMERGENCY: Pause all contracts on all chains
# Usage: bash scripts/emergency-pause.sh
export PATH="$HOME/.foundry/bin:$PATH"
source .env
KEY="${HONE_SHIN_EVM_KEY}"
if [ -z "$KEY" ]; then echo "FATAL: No EVM key"; exit 1; fi

echo "!!! EMERGENCY PAUSE — ALL CHAINS !!!"

for RPC in "https://mainnet.base.org" "https://arb1.arbitrum.io/rpc" "https://mainnet.optimism.io"; do
  echo "Pausing on $RPC..."
  # Pause sale
  cast send 0x752b8cc5165E6f1D3c0dA8DE7cCa8AA008d69e26 "toggleSale(bool)" false --rpc-url "$RPC" --private-key $KEY 2>&1 | grep "status"
  # Freeze claims (set authority to dead address)  
  cast send 0x25E434d38F4dEc7AF2F6f6488BAe34fBc5781D47 "updateSigningAuthority(address)" "0x0000000000000000000000000000000000000001" --rpc-url "$RPC" --private-key $KEY 2>&1 | grep "status"
done

echo "ALL CONTRACTS PAUSED. To resume, redeploy with new contracts."

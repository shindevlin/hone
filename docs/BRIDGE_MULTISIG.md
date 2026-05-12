# BTCPC Bridge Multisig

## Safe Configuration

| Property | Value |
|----------|-------|
| Network | Base Mainnet (chain ID 8453) |
| Type | Gnosis Safe v1.4.1 |
| Threshold | 2-of-3 |
| Signer 1 | `0xD3675710dADF62a7a7bd321b17cA79A1Cd7CF699` |
| Signer 2 | `0x54f3BBb1406dED7eD7ee618fc342A7E9A0B83A2d` |
| Signer 3 | `0xA14f25D98FD5B03c361e2C95f5CbBDd7Ebb96d6c` |

## Deployed Contracts

After running the deployment script, fill in these addresses:

| Contract | Address |
|----------|---------|
| Safe | `0xfB1fd0E05a916aC03c7Dd4DD3DD45Dde1A950aC9` |
| BridgeLock | `0x68dE3cFe9ee7e13383cf5A9FfFE8ddE457BD5942` |
| BridgeReserve | `0xaDdA7318dDA1FB98A7A4Ef942B264dF1eF81A791` |

## Deployment

### 1. Predict the Safe address (before spending gas)

```bash
node contracts/deploy/simulate_safe_address.js
```

### 2. Deploy Safe + BridgeLock + BridgeReserve

```bash
export DEPLOYER_PRIVATE_KEY=0x...    # Shin's key
export RELAYER_ADDRESS=0x...         # BTCPC node relayer address
export BASESCAN_API_KEY=...

forge script contracts/deploy/DeploySafe.s.sol:DeploySafe \
  --rpc-url https://mainnet.base.org \
  --broadcast \
  --verify \
  --etherscan-api-key $BASESCAN_API_KEY \
  -vvvv
```

### 3. Verify contracts (if --verify missed any)

```bash
BRIDGE_LOCK=0x... BRIDGE_RESERVE=0x... SAFE=0x... \
  bash contracts/deploy/verify_bridge.sh
```

### 4. Update .env and node config

```bash
BTCPC_BRIDGE_SAFE=0x...
BTCPC_BRIDGE_LOCK=0x...
BTCPC_BRIDGE_RESERVE=0x...
```

### 5. Fund BridgeReserve

Send initial USDT/USDC/DAI reserves via the Safe UI at `https://app.safe.global/base:<SAFE_ADDRESS>`.

## Safe Operations (post-deploy)

All privileged operations on BridgeLock and BridgeReserve require 2-of-3 Safe signatures:

| Operation | Contract | When |
|-----------|----------|------|
| `setMintedDreams()` | BridgeLock | Oracle updates the cumulative BTCPC minted counter |
| `setAcceptedToken()` | BridgeLock | Add/remove supported stablecoins |
| `unlockFunds()` | BridgeLock | Refund a failed bridge deposit (7-day timelock) |
| `fundToken()` / `fundETH()` | BridgeReserve | Add liquidity for unwrap payouts |
| `setRelayer()` | BridgeReserve | Rotate the trusted relayer address |
| `emergencyWithdraw*()` | BridgeReserve | Emergency liquidity withdrawal |

## Trust Model

See `docs/BRIDGE_TRUST_MODEL.md` for the full security model.

The Safe acts as the single owner of both contracts. No individual key can execute privileged
operations — any action requires at least 2 of the 3 signers to sign a Safe transaction.

The relayer (BTCPC node) can only call `unlock()` on BridgeReserve — it cannot move funds
freely or change contract parameters.

// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import "forge-std/Script.sol";
import "../BridgeLock.sol";
import "../BridgeReserve.sol";

// ── Safe v1.4.1 interfaces (Base mainnet) ─────────────────────────────────
interface ISafeProxyFactory {
    function createProxyWithNonce(
        address singleton,
        bytes memory initializer,
        uint256 saltNonce
    ) external returns (address proxy);
}

interface ISafe {
    function setup(
        address[] calldata owners,
        uint256 threshold,
        address to,
        bytes calldata data,
        address fallbackHandler,
        address paymentToken,
        uint256 payment,
        address payable paymentReceiver
    ) external;
}

contract DeploySafe is Script {
    // Safe v1.4.1 — Base mainnet
    address constant SAFE_FACTORY    = 0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67;
    address constant SAFE_SINGLETON  = 0x29fcB43b46531BcA003ddC8FCB67FFE91900C762;
    address constant FALLBACK_HANDLER = 0xfd0732Dc9E303f09fCEf3a7388Ad10A83459Ec99;

    // HONE bridge multisig — 2-of-3
    address constant SIGNER_1 = 0xD3675710dADF62a7a7bd321b17cA79A1Cd7CF699;
    address constant SIGNER_2 = 0x54f3BBb1406dED7eD7ee618fc342A7E9A0B83A2d;
    address constant SIGNER_3 = 0xA14f25D98FD5B03c361e2C95f5CbBDd7Ebb96d6c;
    uint256 constant THRESHOLD = 2;

    // Existing Base mainnet tokens
    address constant USDT   = 0xfde4C96c8593536E31F229EA8f37b2ADa2699bb2;
    address constant USDC   = 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913;
    address constant DAI    = 0x50c5725949A6F0c72E6C4a641F24049A917DB0Cb;
    address constant WHONE = 0x25E434d38F4dEc7AF2F6f6488BAe34fBc5781D47;

    // Relayer address — set via RELAYER_ADDRESS env var or default to deployer
    address relayer;

    function run() external {
        uint256 deployerKey = vm.envUint("DEPLOYER_PRIVATE_KEY");
        address deployer = vm.addr(deployerKey);
        relayer = vm.envOr("RELAYER_ADDRESS", deployer);

        vm.startBroadcast(deployerKey);

        // ── 1. Deploy Gnosis Safe (2-of-3) ────────────────────────────────
        address safe = _deploySafe();
        console.log("Safe deployed:         ", safe);

        // ── 2. Deploy BridgeLock (deployer owns, then transfer to Safe) ──
        address[] memory initialTokens = new address[](3);
        initialTokens[0] = USDT;
        initialTokens[1] = USDC;
        initialTokens[2] = DAI;
        BridgeLock lock = new BridgeLock(initialTokens);
        console.log("BridgeLock deployed:   ", address(lock));

        // ── 3. Deploy BridgeReserve (deployer owns, then transfer) ───────
        BridgeReserve reserve = new BridgeReserve(relayer);
        console.log("BridgeReserve deployed:", address(reserve));

        // ── 4. Transfer ownership of both contracts to the Safe ───────────
        lock.transferOwnership(safe);
        reserve.transferOwnership(safe);
        console.log("Ownership transferred to Safe");

        vm.stopBroadcast();

        // ── 5. Print summary ──────────────────────────────────────────────
        console.log("\n=== HONE Bridge Deployment Summary ===");
        console.log("Network:        Base Mainnet (8453)");
        console.log("Safe (2-of-3): ", safe);
        console.log("  Signer 1:    ", SIGNER_1);
        console.log("  Signer 2:    ", SIGNER_2);
        console.log("  Signer 3:    ", SIGNER_3);
        console.log("BridgeLock:    ", address(lock));
        console.log("BridgeReserve: ", address(reserve));
        console.log("Relayer:       ", relayer);
        console.log("\nNext: verify on Basescan, fund BridgeReserve, set HONE_BRIDGE_LOCK and HONE_BRIDGE_RESERVE in .env");
    }

    function _deploySafe() internal returns (address) {
        address[] memory owners = new address[](3);
        owners[0] = SIGNER_1;
        owners[1] = SIGNER_2;
        owners[2] = SIGNER_3;

        bytes memory initializer = abi.encodeCall(
            ISafe.setup,
            (
                owners,
                THRESHOLD,
                address(0),   // no delegate call on setup
                "",           // no data
                FALLBACK_HANDLER,
                address(0),   // no payment token
                0,            // no payment
                payable(address(0))
            )
        );

        // Salt: hash of signers + threshold + hone tag — deterministic address
        uint256 salt = uint256(keccak256(abi.encode(
            SIGNER_1, SIGNER_2, SIGNER_3, THRESHOLD, "hone-bridge-v1"
        )));

        return ISafeProxyFactory(SAFE_FACTORY).createProxyWithNonce(
            SAFE_SINGLETON,
            initializer,
            salt
        );
    }
}

#!/usr/bin/env node
// Compute the deterministic Safe address before deploying.
// Run: node contracts/deploy/simulate_safe_address.js
"use strict";

const { ethers } = require("ethers");

const SAFE_FACTORY   = "0x4e1DCf7AD4e460CfD30791CCC4F9c8a4f820ec67";
const SAFE_SINGLETON = "0x29fcB43b46531BcA003ddC8FCB67FFE91900C762";
const FALLBACK      = "0xfd0732Dc9E303f09fCEf3a7388Ad10A83459Ec99";

const SIGNERS = [
    "0xD3675710dADF62a7a7bd321b17cA79A1Cd7CF699",
    "0x54f3BBb1406dED7eD7ee618fc342A7E9A0B83A2d",
    "0xA14f25D98FD5B03c361e2C95f5CbBDd7Ebb96d6c",
];
const THRESHOLD = 2;

// Safe setup() selector + encoded args
const iface = new ethers.Interface([
    "function setup(address[],uint256,address,bytes,address,address,uint256,address)"
]);
const initializer = iface.encodeFunctionData("setup", [
    SIGNERS, THRESHOLD,
    ethers.ZeroAddress, "0x",
    FALLBACK,
    ethers.ZeroAddress, 0, ethers.ZeroAddress,
]);

// Salt matches DeploySafe.s.sol
const salt = ethers.keccak256(
    ethers.AbiCoder.defaultAbiCoder().encode(
        ["address","address","address","uint256","string"],
        [...SIGNERS, THRESHOLD, "btcpc-bridge-v1"]
    )
);

// Safe uses CREATE2 via factory — proxy bytecode hash is constant for a given singleton
// ProxyCreationCode from Safe factory v1.4.1
const PROXY_CREATION_CODE =
    "0x608060405234801561001057600080fd5b506040516101e63803806101e68339818101604052602081101561003357600080fd5b8101908080519060200190929190505050600073ffffffffffffffffffffffffffffffffffffffff168173ffffffffffffffffffffffffffffffffffffffff1614156100ca576040517f08c379a00000000000000000000000000000000000000000000000000000000081526004018080602001828103825260228152602001806101c46022913960400191505060405180910390fd5b806000806101000a81548173ffffffffffffffffffffffffffffffffffffffff021916908373ffffffffffffffffffffffffffffffffffffffff1602179055505060ab806101196000396000f3fe608060405273ffffffffffffffffffffffffffffffffffffffff600054167fa619486e0000000000000000000000000000000000000000000000000000000060003514156050578060005260206000f35b3660008037600080366000845af43d6000803e60008114156070573d6000fd5b3d6000f3fea264697066735822122003d1488ee65e08fa41e58e888a9865554c535f2c77126a82cb4c0f917f31441364736f6c63430007060033496e76616c69640000000000000000000000000000000000000000000000000000";

const initCodeHash = ethers.keccak256(
    ethers.concat([
        PROXY_CREATION_CODE,
        ethers.AbiCoder.defaultAbiCoder().encode(["address"], [SAFE_SINGLETON])
    ])
);

const create2Salt = ethers.keccak256(
    ethers.concat([
        ethers.keccak256(initializer),
        ethers.zeroPadValue(ethers.toBeHex(BigInt(salt)), 32)
    ])
);

const safeAddress = ethers.getCreate2Address(SAFE_FACTORY, create2Salt, initCodeHash);

console.log("=== BTCPC Bridge Safe (predicted address) ===");
console.log("Safe address:  ", safeAddress);
console.log("Signers (2-of-3):");
SIGNERS.forEach((s, i) => console.log(`  [${i+1}] ${s}`));
console.log("\nVerify this matches after deployment on https://app.safe.global/base:" + safeAddress);

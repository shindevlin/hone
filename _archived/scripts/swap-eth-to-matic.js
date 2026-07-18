#!/usr/bin/env node
"use strict";

/**
 * Swap ETH → MATIC on Uniswap V3 (Ethereum mainnet)
 * Then bridge MATIC from Ethereum → Polygon via PoS bridge
 */

const { ethers } = require("ethers");

const RPC = "https://1rpc.io/eth";
const PRIVATE_KEY = process.env.HONE_SHIN_EVM_KEY;
if (!PRIVATE_KEY) { console.error("Set HONE_SHIN_EVM_KEY in .env"); process.exit(1); }

// Addresses
const WETH = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
const MATIC = "0x7D1AfA7B718fb893dB30A3aBc0Cfc608AaCfeBB0";
const UNISWAP_ROUTER = "0x68b3465833fb72A70ecDF485E0e4C7bD8665Fc45"; // SwapRouter02
const POLYGON_ERC20_BRIDGE = "0xA0c68C638235ee32657e8f720a23ceC1bFc77C77"; // RootChainManager proxy
const POLYGON_ERC20_PREDICATE = "0x40ec5B33f54e0E8A33A975908C5BA1c14e5BbbDf"; // ERC20Predicate

// How much ETH to swap — keep enough for gas (swap + approve + bridge = ~3 txns)
const SWAP_ETH = ethers.parseEther("0.001"); // ~$2 worth, gets us ~8 POL

const UNISWAP_ABI = [
  "function exactInputSingle((address tokenIn, address tokenOut, uint24 fee, address recipient, uint256 amountIn, uint256 amountOutMinimum, uint160 sqrtPriceLimitX96)) external payable returns (uint256 amountOut)",
  "function multicall(uint256 deadline, bytes[] calldata data) external payable returns (bytes[] memory results)"
];

const ERC20_ABI = [
  "function balanceOf(address) view returns (uint256)",
  "function approve(address spender, uint256 amount) returns (bool)",
  "function allowance(address owner, address spender) view returns (uint256)"
];

const BRIDGE_ABI = [
  "function depositFor(address user, address rootToken, bytes depositData) external"
];

async function main() {
  const provider = new ethers.JsonRpcProvider(RPC);
  const wallet = new ethers.Wallet(PRIVATE_KEY, provider);
  const addr = wallet.address;

  console.log(`Wallet: ${addr}`);
  const balance = await provider.getBalance(addr);
  console.log(`ETH balance: ${ethers.formatEther(balance)} ETH`);

  if (balance < SWAP_ETH + ethers.parseEther("0.002")) {
    console.error(`Not enough ETH. Need ${ethers.formatEther(SWAP_ETH)} for swap + ~0.002 for gas`);
    process.exit(1);
  }

  // Step 1: Swap ETH → MATIC on Uniswap V3
  console.log(`\n=== Step 1: Swap ${ethers.formatEther(SWAP_ETH)} ETH → MATIC on Uniswap V3 ===`);

  const router = new ethers.Contract(UNISWAP_ROUTER, UNISWAP_ABI, wallet);

  const swapParams = {
    tokenIn: WETH,
    tokenOut: MATIC,
    fee: 3000, // 0.3% pool
    recipient: addr,
    amountIn: SWAP_ETH,
    amountOutMinimum: 0, // Accept any amount (small swap, not worth MEV)
    sqrtPriceLimitX96: 0
  };

  const swapData = router.interface.encodeFunctionData("exactInputSingle", [swapParams]);
  const deadline = Math.floor(Date.now() / 1000) + 600; // 10 min

  const feeData = await provider.getFeeData();
  console.log(`Gas price: ${ethers.formatUnits(feeData.gasPrice || 0n, "gwei")} gwei`);

  const swapTx = await router.multicall(deadline, [swapData], {
    value: SWAP_ETH,
    gasLimit: 300000
  });
  console.log(`Swap tx: ${swapTx.hash}`);
  const swapReceipt = await swapTx.wait();
  console.log(`Swap confirmed in block ${swapReceipt.blockNumber}`);

  // Check MATIC balance
  const matic = new ethers.Contract(MATIC, ERC20_ABI, wallet);
  const maticBalance = await matic.balanceOf(addr);
  console.log(`MATIC received: ${ethers.formatEther(maticBalance)}`);

  if (maticBalance === 0n) {
    console.error("No MATIC received from swap!");
    process.exit(1);
  }

  // Step 2: Approve MATIC for Polygon bridge
  console.log("\n=== Step 2: Approve MATIC for Polygon PoS Bridge ===");

  const allowance = await matic.allowance(addr, POLYGON_ERC20_PREDICATE);
  if (allowance < maticBalance) {
    const approveTx = await matic.approve(POLYGON_ERC20_PREDICATE, maticBalance, { gasLimit: 60000 });
    console.log(`Approve tx: ${approveTx.hash}`);
    await approveTx.wait();
    console.log("Approved.");
  } else {
    console.log("Already approved.");
  }

  // Step 3: Bridge MATIC to Polygon
  console.log("\n=== Step 3: Bridge MATIC → Polygon via PoS Bridge ===");

  const bridge = new ethers.Contract(POLYGON_ERC20_BRIDGE, BRIDGE_ABI, wallet);
  const depositData = ethers.AbiCoder.defaultAbiCoder().encode(["uint256"], [maticBalance]);

  const bridgeTx = await bridge.depositFor(addr, MATIC, depositData, { gasLimit: 200000 });
  console.log(`Bridge tx: ${bridgeTx.hash}`);
  const bridgeReceipt = await bridgeTx.wait();
  console.log(`Bridge confirmed in block ${bridgeReceipt.blockNumber}`);

  console.log(`\n=== Done! ===`);
  console.log(`${ethers.formatEther(maticBalance)} MATIC bridging to Polygon.`);
  console.log(`Should arrive in ~7-8 minutes (PoS bridge).`);
  console.log(`Polygon address: ${addr}`);

  // Check remaining ETH
  const remaining = await provider.getBalance(addr);
  console.log(`Remaining ETH on mainnet: ${ethers.formatEther(remaining)}`);
}

main().catch(err => {
  console.error("Failed:", err.message);
  process.exit(1);
});

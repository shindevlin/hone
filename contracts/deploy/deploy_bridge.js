// deploy_bridge.js — Hardhat deploy script for BridgeLock + BridgeReserve
//
// Usage:
//   npx hardhat run contracts/deploy/deploy_bridge.js --network base
//   npx hardhat run contracts/deploy/deploy_bridge.js --network base-sepolia
//
// Required env vars (set in .env or hardhat.config.js):
//   DEPLOYER_PRIVATE_KEY  — private key for 0xBDe88F2B3a224B242704bD166804E0E12c75e830
//   RELAYER_ADDRESS       — address of the trusted HONE relayer
//
// Known addresses (Base mainnet):
//   wHONE  : 0x25E434d38F4dEc7AF2F6f6488BAe34fBc5781D47
//   USDT    : 0xfde4C96c8593536E31F229EA8f37b2ADa2699bb2
//   USDC    : 0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913
//   DAI     : 0x50c5725949A6F0c72E6C4a641F24049A917DB0Cb

const hre = require("hardhat");

// ── Configuration ──────────────────────────────────────────────────────────

const DEPLOYER = "0xBDe88F2B3a224B242704bD166804E0E12c75e830";

// Whitelisted stablecoins passed to BridgeLock at deploy time.
// Adjust for the target network if needed.
const INITIAL_TOKENS = {
  "base": [
    "0xfde4C96c8593536E31F229EA8f37b2ADa2699bb2", // USDT (Base)
    "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", // USDC (Base)
    "0x50c5725949A6F0c72E6C4a641F24049A917DB0Cb", // DAI  (Base)
  ],
  "base-sepolia": [
    // Replace with testnet addresses when available
  ],
};

// ── Main ───────────────────────────────────────────────────────────────────

async function main() {
  const network = hre.network.name;
  console.log(`\nDeploying bridge contracts to: ${network}`);

  const [deployer] = await hre.ethers.getSigners();
  console.log(`Deployer address : ${deployer.address}`);
  console.log(`Expected deployer: ${DEPLOYER}`);

  const relayerAddress = process.env.RELAYER_ADDRESS;
  if (!relayerAddress) {
    throw new Error("RELAYER_ADDRESS env var not set");
  }
  console.log(`Relayer address  : ${relayerAddress}`);

  const initialTokens = INITIAL_TOKENS[network] ?? [];
  if (initialTokens.length === 0) {
    console.warn(`Warning: no initial tokens configured for network "${network}".`);
    console.warn("BridgeLock will accept no tokens until setAcceptedToken() is called.");
  }

  // ── Deploy BridgeLock ──────────────────────────────────────────────────

  console.log("\nDeploying BridgeLock...");
  const BridgeLock = await hre.ethers.getContractFactory("BridgeLock");
  const bridgeLock = await BridgeLock.deploy(initialTokens);
  await bridgeLock.waitForDeployment();
  const bridgeLockAddress = await bridgeLock.getAddress();
  console.log(`BridgeLock deployed at: ${bridgeLockAddress}`);

  // ── Deploy BridgeReserve ───────────────────────────────────────────────

  console.log("\nDeploying BridgeReserve...");
  const BridgeReserve = await hre.ethers.getContractFactory("BridgeReserve");
  const bridgeReserve = await BridgeReserve.deploy(relayerAddress);
  await bridgeReserve.waitForDeployment();
  const bridgeReserveAddress = await bridgeReserve.getAddress();
  console.log(`BridgeReserve deployed at: ${bridgeReserveAddress}`);

  // ── Summary ────────────────────────────────────────────────────────────

  console.log("\n────────────────────────────────────────");
  console.log("Deployment complete");
  console.log(`  Network        : ${network}`);
  console.log(`  BridgeLock     : ${bridgeLockAddress}`);
  console.log(`  BridgeReserve  : ${bridgeReserveAddress}`);
  console.log(`  Relayer        : ${relayerAddress}`);
  console.log(`  Accepted tokens: ${initialTokens.join(", ") || "(none)"}`);
  console.log("────────────────────────────────────────\n");

  // Update deployments.json if it exists in contracts/
  try {
    const fs = require("fs");
    const path = require("path");
    const deploymentsPath = path.join(__dirname, "../deployments.json");
    const deployments = JSON.parse(fs.readFileSync(deploymentsPath, "utf8"));
    deployments[network] = deployments[network] ?? {};
    deployments[network].BridgeLock = bridgeLockAddress;
    deployments[network].BridgeReserve = bridgeReserveAddress;
    fs.writeFileSync(deploymentsPath, JSON.stringify(deployments, null, 2));
    console.log(`deployments.json updated (${deploymentsPath})`);
  } catch {
    // Not fatal — deployments.json may not exist yet
  }
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});

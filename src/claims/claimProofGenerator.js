"use strict";

/**
 * claimProofGenerator.js — Generate signed cross-chain claim proofs
 * Shin Devlin
 *
 * After each epoch, the node generates a claim proof for every linked chain.
 * The proof is signed with the node's posting key and can be submitted to
 * the target chain's wBTCPC claim contract.
 */

const crypto = require("crypto");
const { signTransaction } = require("../wallet/keyManager");
const { getCurrentPeriod, getBlockReward } = require("../services/emissionSchedule");

// Cross-chain ratio schedule: 1:1 in period 1, halving each period
const CROSS_CHAIN_RATIOS = {
  1: 1.0,
  2: 0.5,
  3: 0.25,
  4: 0.125,
  5: 0.0625,
  6: 0.03125,
  7: 0.015625,
  8: 0.0078125,
  9: 0.00390625,
  10: 0.001953125,
  11: 0.0009765625
};

/**
 * Get the cross-chain ratio for a given period number.
 * @param {number} periodNumber
 * @returns {number} The cross-chain multiplier (1.0 for period 1, halving each period)
 */
function getCrossChainRatio(periodNumber) {
  if (CROSS_CHAIN_RATIOS[periodNumber] !== undefined) {
    return CROSS_CHAIN_RATIOS[periodNumber];
  }
  // For any period beyond the table, keep halving
  return Math.pow(0.5, periodNumber - 1);
}

/**
 * Generate a signed claim proof for a specific chain and epoch.
 *
 * @param {string} miner         - BTCPC miner account name (e.g. "shindevlin")
 * @param {number} epoch         - Epoch number
 * @param {number} amount        - Native BTCPC reward earned in this epoch
 * @param {string} chain         - Target chain identifier ("base", "hive", "solana", etc.)
 * @param {string} targetWallet  - Wallet address on the target chain
 * @param {string} postingKey    - Hex-encoded posting private key for signing
 * @returns {Object} Signed claim proof
 */
function generateClaimProof(miner, epoch, amount, chain, targetWallet, postingKey) {
  const period = getCurrentPeriod(epoch);
  if (!period) {
    throw new Error("Epoch " + epoch + " is beyond all emission periods");
  }

  const periodNumber = period.period;
  const crossChainRatio = getCrossChainRatio(periodNumber);
  const claimAmount = parseFloat((amount * crossChainRatio).toFixed(8));

  if (claimAmount <= 0) {
    return null;
  }

  // Build the claim proof payload
  const proof = {
    chain: chain,
    miner: miner,
    target_wallet: targetWallet,
    epoch: epoch,
    amount: claimAmount.toFixed(8),
    period: periodNumber,
    cross_chain_ratio: crossChainRatio.toString(),
    linked_at_epoch: 0,
    timestamp: new Date().toISOString()
  };

  // Create a deterministic message hash for signing
  const message = [
    chain,
    miner,
    targetWallet,
    String(epoch),
    claimAmount.toFixed(8),
    String(periodNumber)
  ].join(":");

  const messageHash = crypto.createHash("sha256").update(message).digest();

  // Sign with the posting key
  const sig = signTransaction(messageHash, postingKey);

  proof.proof_signature = sig.signature;
  proof.proof_recovery = sig.recovery;

  return proof;
}

/**
 * Generate claim proofs for all linked chains on a node.
 *
 * @param {string} miner        - BTCPC miner account name
 * @param {number} epoch        - Epoch number
 * @param {number} amount       - Native BTCPC reward earned
 * @param {Object} linkedChains - Map of chain -> wallet address (e.g. { hive: "@shindevlin", base: "0x..." })
 * @param {string} postingKey   - Hex-encoded posting private key
 * @returns {Array<Object>} Array of signed claim proofs
 */
function generateAllClaimProofs(miner, epoch, amount, linkedChains, postingKey) {
  const proofs = [];

  for (const [chain, targetWallet] of Object.entries(linkedChains)) {
    if (!targetWallet) continue;

    try {
      const proof = generateClaimProof(miner, epoch, amount, chain, targetWallet, postingKey);
      if (proof) {
        proofs.push(proof);
      }
    } catch (err) {
      console.error("[BTCPC] Claim proof generation failed for " + chain + ": " + err.message);
    }
  }

  return proofs;
}

module.exports = {
  generateClaimProof,
  generateAllClaimProofs,
  getCrossChainRatio,
  CROSS_CHAIN_RATIOS
};

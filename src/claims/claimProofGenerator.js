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

/**
 * Cross-chain ratio: 10% decay, 2 steps per native period.
 *
 * Each native period has 2 cross-chain steps (first half / second half).
 * Each step multiplies by 0.9 (10% drop).
 *
 * Period 1 first half:  0.9^0 = 100%
 * Period 1 second half: 0.9^1 = 90%
 * Period 2 first half:  0.9^2 = 81%
 * Period 2 second half: 0.9^3 = 72.9%
 * Period 3 first half:  0.9^4 = 65.6%
 * ...
 *
 * Formula: ratio = 0.9 ^ ccStep
 *   where ccStep = (periodNumber - 1) * 2 + (0 if first half, 1 if second half)
 */

/**
 * Get the cross-chain ratio for a given epoch.
 * @param {number} periodNumber - The native emission period
 * @param {number} [epochInPeriod] - How far into the period (0 to period.epochs-1)
 * @param {number} [periodEpochs] - Total epochs in this period
 * @returns {number} The cross-chain multiplier
 */
function getCrossChainRatio(periodNumber, epochInPeriod, periodEpochs) {
  // Calculate which CC step we're on (2 per native period)
  const isSecondHalf = (epochInPeriod != null && periodEpochs != null)
    ? (epochInPeriod >= periodEpochs / 2)
    : false;
  const ccStep = (periodNumber - 1) * 2 + (isSecondHalf ? 1 : 0);
  return parseFloat(Math.pow(0.9, ccStep).toFixed(10));
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
  const epochInPeriod = epoch - period.start_epoch;
  const crossChainRatio = getCrossChainRatio(periodNumber, epochInPeriod, period.epochs);
  const totalClaimAmount = parseFloat((amount * crossChainRatio).toFixed(10));

  if (totalClaimAmount <= 0) {
    return null;
  }

  // Protocol-level 50/50 split: direct to miner + LP pool
  const directAmount = parseFloat((totalClaimAmount * 0.5).toFixed(10));
  const lpAmount = parseFloat((totalClaimAmount - directAmount).toFixed(10));

  // Build the claim proof payload
  const proof = {
    chain: chain,
    miner: miner,
    target_wallet: targetWallet,
    epoch: epoch,
    amount: totalClaimAmount.toFixed(10),
    direct_amount: directAmount.toFixed(10),
    lp_amount: lpAmount.toFixed(10),
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
    totalClaimAmount.toFixed(10),
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
  getCrossChainRatio
};

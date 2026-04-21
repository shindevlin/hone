"use strict";

/**
 * claimProofGenerator.js — Generate signed cross-chain claim proofs
 * Shin Devlin
 *
 * Produces EVM-compatible EIP-191 proofs the wBTCPC contract can verify via
 * ecrecover. The oracle signs with a secp256k1 key whose Ethereum address is
 * set as signingAuthority on the deployed contracts.
 *
 * Non-EVM chains (Solana, TON, etc.) use the same payload but different
 * signature encoding — their verifiers are handled separately.
 *
 * Cross-chain ratio: 10% decay, 2 steps per native emission period.
 *   Period 1 step 0: 0.9^0 = 100%  (first half of period 1)
 *   Period 1 step 1: 0.9^1 = 90%   (second half)
 *   Period 2 step 0: 0.9^2 = 81%   ...
 * Per-chain amount = nativeMiningReward × ratio × 0.1
 * (0.1 = one-tenth per chain; 10 chains × 0.1 = full ratio at step 0)
 */

const crypto   = require("crypto");
const ethers   = require("ethers");
const { getCurrentPeriod } = require("../services/emissionSchedule");

/**
 * Cross-chain decay ratio for a given epoch.
 */
function getCrossChainRatio(periodNumber, epochInPeriod, periodEpochs) {
  const isSecondHalf = (epochInPeriod != null && periodEpochs != null)
    ? (epochInPeriod >= periodEpochs / 2)
    : false;
  const ccStep = (periodNumber - 1) * 2 + (isSecondHalf ? 1 : 0);
  return parseFloat(Math.pow(0.9, ccStep).toFixed(10));
}

/**
 * Generate a nonce-based EVM-compatible claim proof.
 *
 * @param {object} opts
 * @param {string}  opts.btcpcAccount   - BTCPC account name ("shindevlin")
 * @param {string}  opts.chain          - Target chain ("ethereum"|"base"|"arbitrum"|...)
 * @param {string}  opts.targetWallet   - Claimer's EVM address (0x...)
 * @param {number}  opts.amount         - Raw BTCPC credit amount to claim (BTCPC units)
 * @param {string}  opts.nonce          - Unique 32-byte hex nonce from recordCrossChainClaim
 * @param {number}  opts.epoch          - Epoch at which claim was recorded
 * @param {string}  opts.oraclePrivKey  - Hex private key of the oracle signing authority
 *
 * @returns {object} Claim proof ready to submit to wBTCPC.claim()
 */
function generateClaimProof({ btcpcAccount, chain, targetWallet, amount, nonce, epoch, oraclePrivKey }) {
  if (!btcpcAccount) throw new Error("btcpcAccount required");
  if (!targetWallet)  throw new Error("targetWallet required");
  if (!nonce)         throw new Error("nonce required");
  if (!oraclePrivKey) throw new Error("oraclePrivKey required");

  const chainIds = {
    ethereum:  1,
    base:      8453,
    arbitrum:  42161,
    optimism:  10,
    bsc:       56,
    polygon:   137,
  };
  const chainId = chainIds[chain];
  if (!chainId) throw new Error("unsupported EVM chain: " + chain);

  // Convert amount to raw token units (10 decimals)
  const rawAmount = BigInt(Math.round(amount * 1e10));

  // Pack account name to bytes32 (right-padded with zeros)
  const accountBytes = Buffer.alloc(32);
  Buffer.from(btcpcAccount, "utf8").copy(accountBytes, 0, 0, Math.min(btcpcAccount.length, 32));
  const accountBytes32 = "0x" + accountBytes.toString("hex");

  const nonceBytes32 = nonce.startsWith("0x") ? nonce : "0x" + nonce.padEnd(64, "0");

  // keccak256(chainId, claimer, btcpcAccount, amount, nonce) — mirrors the contract
  const msgHash = ethers.solidityPackedKeccak256(
    ["uint256", "address", "bytes32", "uint256", "bytes32"],
    [chainId, targetWallet, accountBytes32, rawAmount, nonceBytes32]
  );

  // EIP-191 sign
  const wallet = new ethers.Wallet(oraclePrivKey.startsWith("0x") ? oraclePrivKey : "0x" + oraclePrivKey);
  const ethSignedHash = ethers.hashMessage(ethers.getBytes(msgHash));
  const sigObj = wallet.signingKey.sign(ethSignedHash);
  const signature = ethers.Signature.from(sigObj).serialized;

  return {
    chain,
    chain_id: chainId,
    btcpc_account: btcpcAccount,
    target_wallet: targetWallet,
    amount_btcpc: amount,
    amount_raw: rawAmount.toString(),
    nonce: nonceBytes32,
    epoch,
    signature,
    signing_authority: wallet.address,
  };
}

/**
 * Generate proofs for all linked EVM chains from accumulated cross-chain credits.
 *
 * @param {object} opts
 * @param {string}  opts.btcpcAccount
 * @param {object}  opts.chainAddresses  - { ethereum: "0x...", base: "0x...", ... }
 * @param {object}  opts.credits         - { ethereum: 5.3, base: 5.3, ... } (from getCrossChainCredits)
 * @param {string}  opts.nonce           - Nonce from recordCrossChainClaim
 * @param {number}  opts.epoch
 * @param {string}  opts.oraclePrivKey
 */
function generateAllClaimProofs({ btcpcAccount, chainAddresses, credits, nonce, epoch, oraclePrivKey }) {
  const proofs = [];
  const evmChains = ["ethereum", "base", "arbitrum", "optimism", "bsc", "polygon"];

  for (const chain of evmChains) {
    const wallet = chainAddresses[chain] || chainAddresses["eth"];
    const amount = credits[chain] || 0;
    if (!wallet || amount <= 0) continue;
    try {
      proofs.push(generateClaimProof({ btcpcAccount, chain, targetWallet: wallet, amount, nonce, epoch, oraclePrivKey }));
    } catch (err) {
      console.error("[BTCPC] Claim proof failed for " + chain + ": " + err.message);
    }
  }
  return proofs;
}

module.exports = { generateClaimProof, generateAllClaimProofs, getCrossChainRatio };

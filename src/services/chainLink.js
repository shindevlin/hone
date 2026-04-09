"use strict";

/**
 * BTCPC Chain Link — Link external wallets to BTCPC accounts
 * Shin Devlin
 *
 * Users prove ownership of external wallets (ETH, Solana, etc.) by
 * signing a challenge message. No private keys are shared — just a
 * signature that proves the user controls the address.
 *
 * Supported chains:
 *   EVM (Ethereum, Base, Arbitrum, Optimism) — secp256k1 + keccak256
 *   Solana — ed25519 (future)
 *   Bitcoin — secp256k1 (future)
 */

var crypto = require("crypto");
var secp256k1 = require("secp256k1");
var { keccak256 } = require("js-sha3");

var LedgerEntry = require("../models/LedgerEntry");

// Active challenges: Map<challengeId, { username, chain, address, challenge, expiresAt }>
var pendingChallenges = new Map();

// ─── Challenge Generation ────────────────────────────────────────

/**
 * Generate a link challenge for a user.
 * The user must sign this exact message with their external wallet.
 *
 * @param {string} username — BTCPC account
 * @param {string} chain — "evm", "solana", "bitcoin"
 * @param {string} address — the external wallet address to link
 * @returns {{ challengeId, message, expiresIn }}
 */
function generateChallenge(username, chain, address) {
  var challengeId = crypto.randomBytes(16).toString("hex");
  var message = "BTCPC-LINK:" + username + ":" + chain + ":" + address + ":" + challengeId;
  var expiresAt = Date.now() + 600000; // 10 minutes

  pendingChallenges.set(challengeId, {
    username: username,
    chain: chain,
    address: address.toLowerCase(),
    message: message,
    expiresAt: expiresAt
  });

  // Cleanup old challenges
  for (var entry of pendingChallenges) {
    if (entry[1].expiresAt < Date.now()) pendingChallenges.delete(entry[0]);
  }

  return {
    challengeId: challengeId,
    message: message,
    expiresIn: 600
  };
}

// ─── EVM Signature Verification ──────────────────────────────────

/**
 * Recover an Ethereum address from a signed message.
 * Implements EIP-191 personal_sign recovery.
 *
 * @param {string} message — the original message
 * @param {string} signature — hex-encoded signature (130 chars = 65 bytes)
 * @returns {string} recovered address (lowercase, 0x-prefixed)
 */
function recoverEVMAddress(message, signature) {
  // EIP-191: "\x19Ethereum Signed Message:\n" + length + message
  var prefix = "\x19Ethereum Signed Message:\n" + message.length + message;
  var msgHash = Buffer.from(keccak256(Buffer.from(prefix)), "hex");

  // Parse signature
  var sigBuf = Buffer.from(signature.replace("0x", ""), "hex");
  if (sigBuf.length !== 65) throw new Error("Signature must be 65 bytes");

  var r = sigBuf.slice(0, 32);
  var s = sigBuf.slice(32, 64);
  var v = sigBuf[64];

  // Normalize v (MetaMask uses 27/28, some wallets use 0/1)
  var recovery = v >= 27 ? v - 27 : v;
  if (recovery !== 0 && recovery !== 1) throw new Error("Invalid recovery byte: " + v);

  var sigOnly = Buffer.concat([r, s]);

  // Recover public key
  var pubKey = secp256k1.ecdsaRecover(sigOnly, recovery, msgHash, false);

  // Address = last 20 bytes of keccak256(pubkey without prefix)
  var pubKeyHash = keccak256(Buffer.from(pubKey.slice(1)));
  var address = "0x" + pubKeyHash.slice(-40);

  return address.toLowerCase();
}

// ─── Verification & Linking ──────────────────────────────────────

/**
 * Verify a signed challenge and link the address to the BTCPC account.
 *
 * @param {string} challengeId
 * @param {string} signature — hex signature from the external wallet
 * @returns {{ success, username, chain, address, error }}
 */
async function verifyAndLink(challengeId, signature) {
  var challenge = pendingChallenges.get(challengeId);
  if (!challenge) {
    return { success: false, error: "Challenge not found or expired" };
  }

  if (Date.now() > challenge.expiresAt) {
    pendingChallenges.delete(challengeId);
    return { success: false, error: "Challenge expired" };
  }

  var recoveredAddress;

  try {
    if (challenge.chain === "evm") {
      recoveredAddress = recoverEVMAddress(challenge.message, signature);
    } else {
      return { success: false, error: "Chain '" + challenge.chain + "' verification not yet supported" };
    }
  } catch (err) {
    return { success: false, error: "Signature verification failed: " + err.message };
  }

  // Check recovered address matches the claimed address
  if (recoveredAddress !== challenge.address.toLowerCase()) {
    return {
      success: false,
      error: "Address mismatch. Expected " + challenge.address + ", recovered " + recoveredAddress
    };
  }

  // Record the link on the permanent ledger
  var ledger = require("./ledger");
  var epoch = await ledger.getCurrentEpoch();

  var entry = new LedgerEntry({
    type: "ACCOUNT_CREATE", // Reuse ACCOUNT_CREATE to update chain_addresses
    from: challenge.username,
    to: challenge.username,
    epoch: epoch,
    memo: "chain-link:" + challenge.chain + ":" + recoveredAddress,
    account_data: {
      username: challenge.username,
      chain_addresses: {
        [challenge.chain]: recoveredAddress
      }
    }
  });
  await entry.save();

  // Also update the User model if it exists
  try {
    var User = require("../models/User");
    var user = await User.findOne({ username: challenge.username });
    if (user) {
      if (!user.linkedAddresses) user.linkedAddresses = {};
      if (!user.linkedAddresses[challenge.chain]) user.linkedAddresses[challenge.chain] = [];
      if (user.linkedAddresses[challenge.chain].indexOf(recoveredAddress) === -1) {
        user.linkedAddresses[challenge.chain].push(recoveredAddress);
      }
      user.markModified("linkedAddresses");
      await user.save();
    }
  } catch (_) {}

  // Cleanup
  pendingChallenges.delete(challengeId);

  return {
    success: true,
    username: challenge.username,
    chain: challenge.chain,
    address: recoveredAddress
  };
}

/**
 * Get all linked addresses for a user.
 */
async function getLinkedAddresses(username) {
  var entries = await LedgerEntry.find({
    from: username,
    memo: { $regex: /^chain-link:/ }
  }).lean();

  var linked = {};
  for (var i = 0; i < entries.length; i++) {
    var parts = (entries[i].memo || "").split(":");
    if (parts.length >= 3) {
      var chain = parts[1];
      var addr = parts[2];
      if (!linked[chain]) linked[chain] = [];
      if (linked[chain].indexOf(addr) === -1) linked[chain].push(addr);
    }
  }

  return linked;
}

module.exports = {
  generateChallenge: generateChallenge,
  verifyAndLink: verifyAndLink,
  recoverEVMAddress: recoverEVMAddress,
  getLinkedAddresses: getLinkedAddresses
};

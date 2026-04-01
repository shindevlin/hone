"use strict";

const crypto = require('crypto');
const User = require('../models/User');
const Transaction = require('../models/Transaction');
const { verifySignature, txHash } = require('../wallet/keyManager');

const CHALLENGE_EXPIRY_MS = 600000; // 10 minutes

/**
 * Start a telegram link — generates a challenge code.
 * User must sign this challenge with their BTCPC posting key.
 */
async function startLink(username, telegramId, telegramUsername) {
  const user = await User.findOne({ username: username.toLowerCase() });
  if (!user) throw new Error('Account not found on BTCPC chain');

  if (user.telegramId && user.telegramId !== telegramId) {
    throw new Error('Account already linked to another Telegram user');
  }
  if (user.telegramId === telegramId) {
    throw new Error('Already linked');
  }

  if (!user.postingPublicKey) {
    throw new Error('Account has no posting key set. Register via CLI first.');
  }

  const challenge = 'BTCPC-VERIFY-' + crypto.randomBytes(8).toString('hex');

  user.pendingTelegramLink = {
    telegramId,
    telegramUsername: telegramUsername || null,
    challenge,
    expiresAt: new Date(Date.now() + CHALLENGE_EXPIRY_MS)
  };
  await user.save();

  return {
    challenge,
    expiresIn: CHALLENGE_EXPIRY_MS / 1000
  };
}

/**
 * Verify a signed challenge to complete Telegram linking.
 * The signature must be produced by the account's posting private key.
 * This proves the Telegram user controls the BTCPC account.
 *
 * @param {string} username - BTCPC username
 * @param {string} telegramId - Telegram user ID (must match pending link)
 * @param {string} signature - Hex-encoded secp256k1 signature of the challenge
 * @param {number} recovery - Signature recovery ID (0 or 1)
 */
async function verifySignedChallenge(username, telegramId, signature, recovery) {
  const user = await User.findOne({ username: username.toLowerCase() });
  if (!user) throw new Error('User not found');

  const pending = user.pendingTelegramLink;
  if (!pending || !pending.challenge) {
    throw new Error('No pending link. Use /link on Telegram first.');
  }
  if (pending.telegramId !== telegramId) {
    throw new Error('Telegram ID does not match the pending link.');
  }
  if (new Date() > pending.expiresAt) {
    user.pendingTelegramLink = {};
    await user.save();
    throw new Error('Challenge expired. Use /link again.');
  }

  // Verify the signature against the posting public key
  if (!user.postingPublicKey) {
    throw new Error('Account has no posting key. Cannot verify.');
  }

  const challenge = pending.challenge;
  const valid = verifySignature(challenge, signature, user.postingPublicKey);
  if (!valid) {
    throw new Error('Invalid signature. Must be signed with the posting key for this account.');
  }

  // Signature valid — record on-chain and link
  const Wallet = require('../models/Wallet');
  const wallet = await Wallet.findOne({ userId: user._id, chain: 'btcpc' });

  const tx = new Transaction({
    from: wallet ? wallet.address : user.username,
    to: 'btcpc_system',
    amount: 0,
    type: 'verification',
    memo: JSON.stringify({
      action: 'verify-telegram',
      challenge: challenge,
      telegram_id: pending.telegramId,
      telegram_username: pending.telegramUsername,
      signature: signature,
      timestamp: new Date().toISOString()
    })
  });
  await tx.save();

  // Link confirmed
  user.telegramId = pending.telegramId;
  user.telegramUsername = pending.telegramUsername;
  user.pendingTelegramLink = {};
  await user.save();

  return {
    success: true,
    username: user.username,
    tx_id: tx._id.toString(),
    message: 'Telegram linked. Posting key signature verified and recorded on-chain.'
  };
}

/**
 * Check if a user has a pending challenge (for bot polling).
 */
async function checkPendingLink(username) {
  const user = await User.findOne({ username: username.toLowerCase() });
  if (!user || !user.pendingTelegramLink?.challenge) return null;
  if (new Date() > user.pendingTelegramLink.expiresAt) return null;
  return user.pendingTelegramLink;
}

module.exports = { startLink, verifySignedChallenge, checkPendingLink };

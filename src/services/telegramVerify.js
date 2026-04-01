"use strict";

const crypto = require('crypto');
const User = require('../models/User');
const { verifySignature } = require('../wallet/keyManager');

const CHALLENGE_EXPIRY_MS = 300000; // 5 minutes

/**
 * Start a telegram link — generates a challenge code.
 * User must sign this with their posting key to prove ownership.
 *
 * @param {string} username - BTCPC username
 * @param {string} telegramId - Telegram user ID
 * @param {string} telegramUsername - Telegram @username
 * @returns {Object} { challenge, expiresIn }
 */
async function startLink(username, telegramId, telegramUsername) {
  const user = await User.findOne({ username: username.toLowerCase() });
  if (!user) throw new Error('Account not found on BTCPC chain');

  if (user.telegramId && user.telegramId !== telegramId) {
    throw new Error('Account already linked to another Telegram user');
  }

  // Already linked to this user
  if (user.telegramId === telegramId) {
    throw new Error('Already linked');
  }

  // Generate challenge
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
    expiresIn: CHALLENGE_EXPIRY_MS / 1000,
    message: `Sign this challenge with your BTCPC posting key to prove you own "${username}".\n\nChallenge: ${challenge}\n\nUse /verify ${username} <signature> to complete.`
  };
}

/**
 * Complete a telegram link by verifying the signed challenge.
 *
 * @param {string} username - BTCPC username
 * @param {string} signature - Hex-encoded signature of the challenge
 * @param {string} telegramId - Telegram user ID (must match pending link)
 * @returns {Object} { success, username }
 */
async function completeLink(username, signature, telegramId) {
  const user = await User.findOne({ username: username.toLowerCase() });
  if (!user) throw new Error('Account not found');

  const pending = user.pendingTelegramLink;
  if (!pending || !pending.challenge) {
    throw new Error('No pending link. Use /link <username> first.');
  }

  if (pending.telegramId !== telegramId) {
    throw new Error('Telegram ID mismatch. Start /link again.');
  }

  if (new Date() > pending.expiresAt) {
    user.pendingTelegramLink = {};
    await user.save();
    throw new Error('Challenge expired. Use /link <username> to get a new one.');
  }

  // Verify signature against posting public key
  const postingPubKey = user.postingPublicKey;
  if (!postingPubKey) {
    // If no posting key registered, fall back to challenge-code-only verification
    // (user types the challenge back as proof they initiated the link)
    if (signature === pending.challenge) {
      user.telegramId = pending.telegramId;
      user.telegramUsername = pending.telegramUsername;
      user.pendingTelegramLink = {};
      await user.save();
      return { success: true, username: user.username, method: 'challenge-echo' };
    }
    throw new Error('Invalid verification. Paste the exact challenge code, or register your posting public key first.');
  }

  // Cryptographic verification
  try {
    const valid = verifySignature(pending.challenge, signature, postingPubKey);
    if (!valid) throw new Error('Invalid signature');
  } catch (err) {
    throw new Error('Signature verification failed: ' + err.message);
  }

  // Link confirmed
  user.telegramId = pending.telegramId;
  user.telegramUsername = pending.telegramUsername;
  user.pendingTelegramLink = {};
  await user.save();

  return { success: true, username: user.username, method: 'signature' };
}

module.exports = { startLink, completeLink };

"use strict";

const crypto = require('crypto');
const User = require('../models/User');
const Transaction = require('../models/Transaction');

const CHALLENGE_EXPIRY_MS = 600000; // 10 minutes — user needs time to post on-chain

/**
 * Start a telegram link — generates a challenge code.
 * User must post this challenge on the BTCPC blockchain with their posting key.
 * The bot watches the chain for matching transactions.
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
 * Post the verification transaction on-chain.
 * Called via API: POST /api/user/verify-telegram
 * Requires JWT auth (proves they hold the account credentials).
 * The transaction itself is recorded on the blockchain.
 */
async function postVerification(userId, challenge) {
  const user = await User.findById(userId);
  if (!user) throw new Error('User not found');

  const pending = user.pendingTelegramLink;
  if (!pending || !pending.challenge) {
    throw new Error('No pending link. Use /link on Telegram first.');
  }
  if (challenge !== pending.challenge) {
    throw new Error('Challenge does not match');
  }
  if (new Date() > pending.expiresAt) {
    user.pendingTelegramLink = {};
    await user.save();
    throw new Error('Challenge expired. Use /link again.');
  }

  // Record the verification ON CHAIN as a transaction
  const Wallet = require('../models/Wallet');
  const wallet = await Wallet.findOne({ userId: user._id, chain: 'btcpc' });

  const tx = new Transaction({
    from: wallet ? wallet.address : user.username,
    to: 'btcpc_system',
    amount: 0,
    type: 'verification',
    memo: JSON.stringify({
      action: 'verify-telegram',
      challenge: pending.challenge,
      telegram_id: pending.telegramId,
      telegram_username: pending.telegramUsername,
      timestamp: new Date().toISOString()
    })
  });
  await tx.save();

  // Link confirmed — the on-chain transaction IS the proof
  user.telegramId = pending.telegramId;
  user.telegramUsername = pending.telegramUsername;
  user.pendingTelegramLink = {};
  await user.save();

  return {
    success: true,
    username: user.username,
    tx_id: tx._id.toString(),
    message: 'Telegram linked. Verification recorded on-chain.'
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

module.exports = { startLink, postVerification, checkPendingLink };

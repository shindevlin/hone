"use strict";
const express = require('express');
const router = express.Router();
const crypto = require('crypto');
const { authenticateToken } = require('../middlewares/auth');
const User = require('../models/User');
const Project = require('../models/Project');
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');

const FAUCET_AMOUNT = 1; // 1 BTCPC per claim
const FAUCET_ADDRESS = 'btcpc_faucet';

/**
 * POST /api/faucet/claim
 * Grants BTCPC to accounts that have either:
 * 1. First-time claim with zero balance (new user onboarding)
 * 2. Spent their previous tokens (balance is zero, have transaction history)
 * 3. Contributed to the network via a verified project
 */
router.post('/claim', authenticateToken, async (req, res) => {
  const userId = req.user.id;

  try {
    const user = await User.findById(userId);
    if (!user) return res.status(404).json({ error: 'User not found' });

    // Balance check from stateStore (chain state)
    const currentBalance = stateStore.getBalance(user.username, 'BTCPC');

    // Check if user still has unspent tokens
    if (currentBalance > 0) {
      return res.status(400).json({
        error: 'You still have BTCPC. Use your tokens before claiming more.',
        balance: currentBalance
      });
    }

    // Check for prior faucet claim via stateStore balance history
    // Simple rule: if balance has ever been above 0 (non-genesis), check for project
    const hasPriorBalance = stateStore.getAccount ? stateStore.getAccount(user.username) : null;
    const hasVerifiedProject = await Project.findOne({ owner: user.username, verified: true });

    // First claim is free
    if (!hasPriorBalance) {
      return await grantFaucet(user.username, currentBalance, 'Welcome to BTCPC — starter tokens', res);
    }

    if (hasVerifiedProject) {
      return await grantFaucet(user.username, currentBalance, `Refill — verified project: ${hasVerifiedProject.name}`, res);
    }

    return res.status(403).json({
      error: 'Faucet requires contribution. Register and verify a project on GitHub to claim more tokens.',
    });

  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

async function grantFaucet(username, currentBalance, memo, res) {
  // Record on permanent ledger
  const epoch = await ledger.getCurrentEpoch();
  await ledger.recordFaucet(username, FAUCET_AMOUNT, epoch);

  res.json({
    success: true,
    amount: FAUCET_AMOUNT,
    balance: currentBalance + FAUCET_AMOUNT,
    username,
    message: `Received ${FAUCET_AMOUNT} BTCPC. ${memo}`
  });
}

module.exports = router;

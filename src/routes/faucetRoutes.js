"use strict";
const express = require('express');
const router = express.Router();
const crypto = require('crypto');
const { authenticateToken } = require('../middlewares/auth');
const User = require('../models/User');
const Wallet = require('../models/Wallet');
const Transaction = require('../models/Transaction');
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

    // Find or create btcpc wallet
    let wallet = await Wallet.findOne({ userId, chain: 'btcpc' });
    if (!wallet) {
      wallet = new Wallet({
        userId,
        chain: 'btcpc',
        address: 'btcpc_' + crypto.randomBytes(20).toString('hex'),
        balance: new Map([['BTCPC', 0]])
      });
      await wallet.save();
    }

    // Balance check from stateStore (chain state), not the Mongo Wallet cache
    const currentBalance = stateStore.getBalance(user.username, 'BTCPC');

    // Check if user still has unspent tokens
    if (currentBalance > 0) {
      return res.status(400).json({
        error: 'You still have BTCPC. Use your tokens before claiming more.',
        balance: currentBalance
      });
    }

    // Count previous faucet claims
    const faucetClaims = await Transaction.countDocuments({
      to: wallet.address,
      type: 'faucet'
    });

    // First claim is free — no requirements
    if (faucetClaims === 0) {
      return await grantFaucet(wallet, currentBalance, 'Welcome to BTCPC — starter tokens', res);
    }

    // Subsequent claims require contribution: verified project or spent tokens on inference
    const hasVerifiedProject = await Project.findOne({
      owner: user.username,
      verified: true
    });

    const hasSpentOnInference = await Transaction.findOne({
      from: wallet.address,
      type: { $in: ['transfer', 'inference'] }
    });

    if (hasVerifiedProject) {
      return await grantFaucet(wallet, currentBalance, `Refill — verified project: ${hasVerifiedProject.name}`, res);
    }

    if (hasSpentOnInference) {
      return await grantFaucet(wallet, currentBalance, 'Refill — tokens spent on network usage', res);
    }

    return res.status(403).json({
      error: 'Faucet requires contribution. Either register and verify a project on GitHub, or spend your tokens using the network first.',
      claims_so_far: faucetClaims
    });

  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

async function grantFaucet(wallet, currentBalance, memo, res) {
  // Resolve username for ledger
  const user = await User.findById(wallet.userId);
  const username = user?.username || wallet.address;

  // Record on permanent ledger
  const epoch = await ledger.getCurrentEpoch();
  await ledger.recordFaucet(username, FAUCET_AMOUNT, epoch);

  // Update wallet cache
  wallet.balance.set('BTCPC', currentBalance + FAUCET_AMOUNT);
  await wallet.save();

  // Record transaction (legacy index)
  const tx = new Transaction({
    from: FAUCET_ADDRESS,
    to: wallet.address,
    amount: FAUCET_AMOUNT,
    type: 'faucet',
    memo
  });
  await tx.save();

  res.json({
    success: true,
    amount: FAUCET_AMOUNT,
    balance: currentBalance + FAUCET_AMOUNT,
    address: wallet.address,
    message: `Received ${FAUCET_AMOUNT} BTCPC. ${memo}`
  });
}

module.exports = router;

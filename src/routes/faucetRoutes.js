"use strict";
const express = require('express');
const router = express.Router();
const crypto = require('crypto');
const { authenticateToken } = require('../middlewares/auth');
const User = require('../models/User');
const Wallet = require('../models/Wallet');
const Transaction = require('../models/Transaction');

const FAUCET_AMOUNT = 1; // 1 BTCPC per claim
const FAUCET_ADDRESS = 'btcpc_faucet';

/**
 * POST /api/faucet/claim
 * Grants starter BTCPC to new accounts (one-time, requires auth)
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
    }

    // Check if already claimed (any faucet transaction to this wallet)
    const alreadyClaimed = await Transaction.findOne({
      to: wallet.address,
      type: 'faucet'
    });
    if (alreadyClaimed) {
      return res.status(400).json({ error: 'Faucet already claimed. Each account gets one free claim.' });
    }

    // Credit the wallet
    const currentBalance = wallet.balance.get('BTCPC') || 0;
    wallet.balance.set('BTCPC', currentBalance + FAUCET_AMOUNT);
    await wallet.save();

    // Record the transaction
    const tx = new Transaction({
      from: FAUCET_ADDRESS,
      to: wallet.address,
      amount: FAUCET_AMOUNT,
      type: 'faucet',
      memo: 'Welcome to BTCPC — starter tokens'
    });
    await tx.save();

    res.json({
      success: true,
      amount: FAUCET_AMOUNT,
      balance: currentBalance + FAUCET_AMOUNT,
      address: wallet.address,
      message: `Received ${FAUCET_AMOUNT} BTCPC. Start using inference or stake to earn more.`
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;

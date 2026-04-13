"use strict";
const express = require('express');
const router = express.Router();
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');

const FAUCET_AMOUNT = 1; // 1 BTCPC per claim
const FAUCET_ADDRESS = 'btcpc_treasury';

/**
 * POST /api/faucet/claim
 * Grants 1 BTCPC to any account with zero balance.
 * No auth required — self-heal onboarding. One claim per account.
 * Body: { account: "username" }
 */
router.post('/claim', async (req, res) => {
  try {
    const account = req.body && req.body.account;
    if (!account || typeof account !== 'string') {
      return res.status(400).json({ error: 'account required' });
    }

    // Must be a known account on chain
    const acc = stateStore.getAccount(account);
    if (!acc) {
      return res.status(404).json({ error: 'account not found on chain' });
    }

    // Only claim if balance is 0
    const balance = stateStore.getBalance(account, 'BTCPC');
    if (balance > 0) {
      return res.status(400).json({
        error: 'You already have ' + balance.toFixed(4) + ' BTCPC. Use your tokens before claiming more.',
        balance: balance
      });
    }

    // Transfer from faucet source to account
    // recordTransfer(from, to, amount, token, signature, epoch, memo)
    await ledger.recordTransfer(FAUCET_ADDRESS, account, FAUCET_AMOUNT, 'BTCPC',
      null, stateStore.getChainHeight(), 'Faucet claim — welcome to BTCPC');

    const newBalance = stateStore.getBalance(account, 'BTCPC');
    res.json({
      success: true,
      amount: FAUCET_AMOUNT,
      balance: newBalance,
      message: 'Welcome to BTCPC — ' + FAUCET_AMOUNT + ' BTCPC granted'
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/faucet/status
 * Public — check faucet balance and claim amount.
 */
router.get('/status', (req, res) => {
  const faucetBalance = stateStore.getBalance(FAUCET_ADDRESS, 'BTCPC');
  res.json({
    faucet_address: FAUCET_ADDRESS,
    balance: faucetBalance,
    claim_amount: FAUCET_AMOUNT,
  });
});

module.exports = router;

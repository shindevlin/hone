"use strict";
const express = require('express');
const router = express.Router();
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');

// Faucet gives enough for ~10 standard inference questions, scales with network maturity.
// As token value grows, the amount decreases but the utility stays the same:
// "here's enough to try BTCPC — ask 10 questions, then earn more or buy more."
function getFaucetAmount() {
  const epoch = stateStore.getChainHeight();
  if (epoch <= 1000) return 1;      // bootstrap: generous, ~1000 small queries
  if (epoch <= 10000) return 0.1;   // growth: ~100 small queries
  return 0.01;                       // mature: ~10 small queries (the target)
}
const FAUCET_ADDRESS = 'btcpc_treasury';

// Anti-spam: track claims per IP
const claimsByIP = new Map(); // ip → { count, lastClaim }
const MAX_CLAIMS_PER_IP = 3; // max 3 accounts per IP per day
const CLAIM_WINDOW_MS = 24 * 60 * 60 * 1000; // 24 hours

function checkIPLimit(ip) {
  const now = Date.now();
  const record = claimsByIP.get(ip);
  if (!record || (now - record.lastClaim) > CLAIM_WINDOW_MS) {
    claimsByIP.set(ip, { count: 1, lastClaim: now });
    return true;
  }
  if (record.count >= MAX_CLAIMS_PER_IP) return false;
  record.count++;
  record.lastClaim = now;
  return true;
}

// Clean old IP records every hour
setInterval(() => {
  const cutoff = Date.now() - CLAIM_WINDOW_MS;
  for (const [ip, rec] of claimsByIP) {
    if (rec.lastClaim < cutoff) claimsByIP.delete(ip);
  }
}, 60 * 60 * 1000);

/**
 * POST /api/faucet/claim
 * Grants BTCPC to any account with zero balance.
 * Anti-spam: max 3 claims per IP per day, one claim per account.
 * Body: { account: "username" }
 */
router.post('/claim', async (req, res) => {
  try {
    const account = req.body && req.body.account;
    if (!account || typeof account !== 'string') {
      return res.status(400).json({ error: 'account required' });
    }

    // Anti-spam: IP rate limit
    const ip = req.ip || req.connection.remoteAddress || 'unknown';
    if (!checkIPLimit(ip)) {
      return res.status(429).json({
        error: 'Too many faucet claims from this IP. Max 3 per day. Earn BTCPC by running sensors or mining.'
      });
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
    await ledger.recordTransfer(FAUCET_ADDRESS, account, getFaucetAmount(), 'BTCPC',
      null, stateStore.getChainHeight(), 'Faucet claim — welcome to BTCPC');

    const newBalance = stateStore.getBalance(account, 'BTCPC');
    res.json({
      success: true,
      amount: getFaucetAmount(),
      balance: newBalance,
      message: 'Welcome to BTCPC — ' + getFaucetAmount() + ' BTCPC granted'
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
    claim_amount: getFaucetAmount(),
  });
});

module.exports = router;

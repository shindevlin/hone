"use strict";
const crypto = require('crypto');
const User = require('../models/User');
const ledger = require('../services/ledger');
const p2p = require('../p2p/network');
const { createTransactionMessage } = require('../p2p/protocol');
const stateStore = require('../chain/stateStore');
const { rejectObjectInputs, sanitizeString, sanitizeAmount, validAddress, validChain } = require('../middlewares/validate');

/**
 * Create Wallet for Authenticated User
 *
 * Phase E: Wallet Mongoose model removed. Wallet records are created during
 * account creation (accountManager.js still uses Wallet for initial setup,
 * which is acceptable). This controller no longer creates Wallet documents.
 */
async function createWallet(req, res) {
  const chain = sanitizeString(req.body.chain, 20) || 'hive';

  try {
    if (typeof req.body.chain === 'object' && req.body.chain !== null) {
      return res.status(400).json({ error: 'chain must be a string' });
    }
    if (!validChain(chain)) return res.status(400).json({ error: 'unsupported chain' });

    const user = await User.findById(req.user.id);
    if (!user) return res.status(404).json({ error: 'User not found' });

    // Address is derived from username (BTCPC chain uses username as identity)
    const address = 'BTCPC' + crypto.createHash('sha256').update(user.username + chain).digest('hex').slice(0, 40);

    res.status(201).json({
      success: true,
      wallet: {
        address,
        chain,
        balance: { BTCPC: stateStore.getBalance(user.username, 'BTCPC') }
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Get Wallet Balance for Authenticated User
 */
async function getBalance(req, res) {
  try {
    const user = await User.findById(req.user.id);
    if (!user) return res.status(404).json({ error: 'User not found' });

    const username = user.username;
    const btcpcBalance = stateStore.getBalance(username, 'BTCPC');
    const tokenBalances = stateStore.getTokenBalances ? stateStore.getTokenBalances(username) : { BTCPC: btcpcBalance };
    if (tokenBalances.BTCPC === undefined) tokenBalances.BTCPC = btcpcBalance;
    const delegatedBalance = stateStore.getDelegatedBalance ? stateStore.getDelegatedBalance(username, 'BTCPC') : 0;

    // Generate deterministic address from username for display
    const address = 'BTCPC' + crypto.createHash('sha256').update(username).digest('hex').slice(0, 40);

    res.json({
      success: true,
      address,
      chain: 'btcpc',
      balance: tokenBalances,
      delegated_balance: delegatedBalance,
      delegated_note: delegatedBalance > 0 ? 'Delegated tokens are inference-only and cannot be transferred.' : undefined,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Transfer BTCPC tokens to another account
 */
async function transfer(req, res) {
  try {
    const objErr = rejectObjectInputs(req.body, ['toAddress', 'amount', 'memo']);
    if (objErr) return res.status(400).json({ error: objErr });
    const toAddress = sanitizeString(req.body.toAddress, 200);
    const amount = sanitizeAmount(req.body.amount);
    const memo = sanitizeString(req.body.memo, 500) || null;
    if (!toAddress || !amount) {
      return res.status(400).json({ error: 'toAddress and amount are required' });
    }
    if (!validAddress(toAddress)) {
      return res.status(400).json({ error: 'invalid address format' });
    }

    const senderUser = await User.findById(req.user.id);
    if (!senderUser) return res.status(404).json({ error: 'Sender not found' });
    const senderName = senderUser.username;

    // Validate sufficient balance via stateStore (O(1) in-memory)
    const senderBalance = stateStore.getBalance(senderName, 'BTCPC');
    if (senderBalance < amount) {
      return res.status(400).json({ error: 'Insufficient BTCPC balance' });
    }

    // Resolve recipient — stateStore is the source of truth (blockchain-only)
    let recipientName = null;
    if (!toAddress.startsWith('BTCPC')) {
      // Treat as username; confirm account exists on chain
      const acct = stateStore.getAccount ? stateStore.getAccount(toAddress) : null;
      if (acct) recipientName = toAddress;
    }
    if (!recipientName) {
      // Unknown — allow forward-sending: tokens held at username until they register
      // (BTCPC account names are reserved on first send, no Mongo lookup required)
      if (!toAddress.startsWith('BTCPC') && toAddress.length >= 3) {
        recipientName = toAddress; // tokens will land when account registers on chain
      } else {
        return res.status(404).json({ error: 'Recipient not found. Use a username or registered address.' });
      }
    }

    // Prevent self-transfer
    if (senderName === recipientName) {
      return res.status(400).json({ error: 'Cannot transfer to your own wallet' });
    }

    // Record on permanent ledger
    const epoch = await ledger.getCurrentEpoch();
    const entry = await ledger.recordTransfer(senderName, recipientName, amount, 'BTCPC', null, epoch, memo || null);
    const txHash = require('../chain/blockStore').hashLedgerEntry(entry && entry.toObject ? entry.toObject() : entry);

    // Broadcast to P2P network
    try {
      const tx = {
        type: 'TRANSFER', from: senderName, to: recipientName,
        amount, token: 'BTCPC', epoch, nonce: Date.now(),
        timestamp: Date.now(), memo: memo || null, txHash
      };
      const txMsg = createTransactionMessage(tx, p2p.NODE_ID);
      p2p.broadcast(txMsg);
    } catch (_) {
      // Non-fatal
    }

    res.json({
      success: true,
      txHash,
      transaction: {
        txHash,
        from: senderName,
        to: recipientName,
        amount,
        type: 'transfer',
        memo: memo || null,
        timestamp: entry && entry.timestamp ? entry.timestamp : new Date().toISOString()
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Get Transaction History for Authenticated User's Wallet
 * Phase E: Transaction model removed — returns ledger entries from stateStore.
 */
async function getTransactionHistory(req, res) {
  try {
    const user = await User.findById(req.user.id);
    if (!user) return res.status(404).json({ error: 'User not found' });

    // Return recent ledger entries for this user from stateStore
    // Full history requires block replay — stateStore is the live cache
    const balance = stateStore.getBalance(user.username, 'BTCPC');
    const address = 'BTCPC' + crypto.createHash('sha256').update(user.username).digest('hex').slice(0, 40);

    res.json({
      success: true,
      address,
      transactions: [],
      note: 'Full history available via block replay. Current balance: ' + balance + ' BTCPC'
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

module.exports = {
  createWallet,
  getBalance,
  transfer,
  getTransactionHistory
};

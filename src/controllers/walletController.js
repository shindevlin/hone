"use strict";
const crypto = require('crypto');
const Wallet = require('../models/Wallet');
const Transaction = require('../models/Transaction');
const User = require('../models/User');
const ledger = require('../services/ledger');
const mempool = require('../p2p/mempool');
const p2p = require('../p2p/network');
const { createTransactionMessage } = require('../p2p/protocol');

/**
 * Create Wallet for Authenticated User
 */
async function createWallet(req, res) {
  const userId = req.user.id;
  const { chain } = req.body;

  try {
    // Check if user already has a wallet for this chain
    const existing = await Wallet.findOne({ userId, chain: chain || 'hive' });
    if (existing) {
      return res.status(400).json({ error: 'Wallet already exists for this chain' });
    }

    // Generate a unique wallet address
    const address = 'urs_' + crypto.randomBytes(20).toString('hex');

    const wallet = new Wallet({
      userId,
      chain: chain || 'hive',
      address,
      balance: new Map([['BTCPC', 0]])
    });

    await wallet.save();

    res.status(201).json({
      success: true,
      wallet: {
        address: wallet.address,
        chain: wallet.chain,
        balance: Object.fromEntries(wallet.balance)
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
  const userId = req.user.id;

  try {
    const wallet = await Wallet.findOne({ userId });
    if (!wallet) {
      return res.status(404).json({ error: 'Wallet not found' });
    }

    res.json({
      success: true,
      address: wallet.address,
      chain: wallet.chain,
      balance: Object.fromEntries(wallet.balance)
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Transfer URS Tokens to Another User
 */
async function transfer(req, res) {
  const userId = req.user.id;
  const { toAddress, amount, memo } = req.body;

  try {
    if (!toAddress || !amount) {
      return res.status(400).json({ error: 'toAddress and amount are required' });
    }

    if (amount <= 0) {
      return res.status(400).json({ error: 'Amount must be greater than zero' });
    }

    // Find sender wallet
    const senderWallet = await Wallet.findOne({ userId });
    if (!senderWallet) {
      return res.status(404).json({ error: 'Sender wallet not found' });
    }

    // Validate sufficient balance
    const senderBalance = senderWallet.balance.get('BTCPC') || 0;
    if (senderBalance < amount) {
      return res.status(400).json({ error: 'Insufficient URS balance' });
    }

    // Find recipient wallet
    const recipientWallet = await Wallet.findOne({ address: toAddress });
    if (!recipientWallet) {
      return res.status(404).json({ error: 'Recipient wallet not found' });
    }

    // Prevent self-transfer
    if (senderWallet.address === toAddress) {
      return res.status(400).json({ error: 'Cannot transfer to your own wallet' });
    }

    // Resolve usernames for ledger
    const senderUser = await User.findById(userId);
    const recipientUser = await User.findOne({
      $or: [
        { _id: recipientWallet.userId }
      ]
    });
    const senderName = senderUser?.username || senderWallet.address;
    const recipientName = recipientUser?.username || recipientWallet.address;

    // Check effective balance (ledger balance minus pending mempool debits)
    const pendingDebit = mempool.getPendingDebit(senderName);
    if (senderBalance - pendingDebit < amount) {
      return res.status(400).json({
        error: 'Insufficient balance (pending transactions)',
        balance: senderBalance,
        pending: pendingDebit,
        available: senderBalance - pendingDebit
      });
    }

    // Build transaction and submit to mempool
    const epoch = await ledger.getCurrentEpoch();
    const tx = {
      type: 'TRANSFER',
      from: senderName,
      to: recipientName,
      amount,
      token: 'BTCPC',
      epoch,
      nonce: Date.now(),
      timestamp: Date.now(),
      memo: memo || null,
      signature: null
    };

    const result = mempool.submit(tx);
    if (!result.accepted) {
      return res.status(400).json({ error: 'Transaction rejected: ' + result.reason });
    }

    // Record on permanent ledger — this IS the chain
    await ledger.recordTransfer(senderName, recipientName, amount, 'BTCPC', null, epoch, memo || null);

    // Update wallet caches immediately — sender sees reduced balance
    senderWallet.balance.set('BTCPC', senderBalance - amount);
    const recipientBalance = recipientWallet.balance.get('BTCPC') || 0;
    recipientWallet.balance.set('BTCPC', recipientBalance + amount);
    await senderWallet.save();
    await recipientWallet.save();

    // Broadcast to P2P network — all nodes see this immediately
    try {
      const txMsg = createTransactionMessage(tx, p2p.NODE_ID);
      p2p.broadcast(txMsg);
    } catch (_) {
      // Non-fatal: tx is on the ledger, will propagate at epoch finalization
    }

    // Record transaction (legacy index)
    const transaction = new Transaction({
      from: senderWallet.address,
      to: toAddress,
      amount,
      type: 'transfer',
      memo: memo || null
    });
    await transaction.save();

    res.json({
      success: true,
      txHash: result.txHash,
      transaction: {
        txHash: result.txHash,
        from: senderName,
        to: recipientName,
        amount,
        type: 'transfer',
        memo: memo || null,
        timestamp: tx.timestamp
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Get Transaction History for Authenticated User's Wallet
 */
async function getTransactionHistory(req, res) {
  const userId = req.user.id;

  try {
    const wallet = await Wallet.findOne({ userId });
    if (!wallet) {
      return res.status(404).json({ error: 'Wallet not found' });
    }

    const transactions = await Transaction.find({
      $or: [{ from: wallet.address }, { to: wallet.address }]
    }).sort({ timestamp: -1 });

    res.json({
      success: true,
      address: wallet.address,
      transactions
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

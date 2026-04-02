"use strict";
const crypto = require('crypto');
const Wallet = require('../models/Wallet');
const Transaction = require('../models/Transaction');

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

    // Execute transfer
    senderWallet.balance.set('BTCPC', senderBalance - amount);
    const recipientBalance = recipientWallet.balance.get('BTCPC') || 0;
    recipientWallet.balance.set('BTCPC', recipientBalance + amount);

    await senderWallet.save();
    await recipientWallet.save();

    // Record transaction
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
      transaction: {
        from: transaction.from,
        to: transaction.to,
        amount: transaction.amount,
        type: transaction.type,
        memo: transaction.memo,
        timestamp: transaction.timestamp
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

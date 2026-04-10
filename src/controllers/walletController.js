"use strict";
const crypto = require('crypto');
const Wallet = require('../models/Wallet');
const Transaction = require('../models/Transaction');
const User = require('../models/User');
const ledger = require('../services/ledger');
const p2p = require('../p2p/network');
const { createTransactionMessage } = require('../p2p/protocol');
const stateStore = require('../chain/stateStore');
const { rejectObjectInputs, sanitizeString, sanitizeAmount, validAddress, validChain } = require('../middlewares/validate');

/**
 * Create Wallet for Authenticated User
 */
async function createWallet(req, res) {
  const userId = req.user.id;
  const chain = sanitizeString(req.body.chain, 20) || 'hive';

  try {
    if (typeof req.body.chain === 'object' && req.body.chain !== null) {
      return res.status(400).json({ error: 'chain must be a string' });
    }
    if (!validChain(chain)) return res.status(400).json({ error: 'unsupported chain' });
    const existing = await Wallet.findOne({ userId, chain });
    if (existing) {
      return res.status(400).json({ error: 'Wallet already exists for this chain' });
    }

    // Generate a unique wallet address
    const address = 'BTCPC' + crypto.randomBytes(20).toString('hex');

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
    // Phase C: still look up the Wallet document for the address — stateStore
    // doesn't index addresses → usernames yet. Balance comes from stateStore.
    const wallet = await Wallet.findOne({ userId });
    if (!wallet) {
      return res.status(404).json({ error: 'Wallet not found' });
    }

    // Resolve username from userId so we can hit stateStore
    const user = await User.findById(userId);
    const username = user && user.username;
    const btcpcBalance = username ? stateStore.getBalance(username, 'BTCPC') : 0;
    const tokenBalances = username ? stateStore.getTokenBalances(username) : { BTCPC: btcpcBalance };
    if (tokenBalances.BTCPC === undefined) tokenBalances.BTCPC = btcpcBalance;

    res.json({
      success: true,
      address: wallet.address,
      chain: wallet.chain,
      balance: tokenBalances
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Transfer BTCPC tokens to another account
 */
async function transfer(req, res) {
  const userId = req.user.id;

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

    // Find sender wallet (still need address for self-transfer check)
    const senderWallet = await Wallet.findOne({ userId });
    if (!senderWallet) {
      return res.status(404).json({ error: 'Sender wallet not found' });
    }

    // Resolve sender username first — balance comes from stateStore, not Wallet
    const senderUser = await User.findById(userId);
    const senderName = senderUser?.username || senderWallet.address;

    // Validate sufficient balance via stateStore (O(1) in-memory)
    const senderBalance = stateStore.getBalance(senderName, 'BTCPC');
    if (senderBalance < amount) {
      return res.status(400).json({ error: 'Insufficient BTCPC balance' });
    }

    // Find recipient wallet (TODO Phase D: add address→username index in stateStore
    // so we can drop this Mongo read. Kept for now because stateStore only keys
    // balances by username.)
    const recipientWallet = await Wallet.findOne({ address: toAddress });
    if (!recipientWallet) {
      return res.status(404).json({ error: 'Recipient wallet not found' });
    }

    // Prevent self-transfer
    if (senderWallet.address === toAddress) {
      return res.status(400).json({ error: 'Cannot transfer to your own wallet' });
    }

    // Resolve recipient username for ledger
    const recipientUser = await User.findOne({
      $or: [
        { _id: recipientWallet.userId }
      ]
    });
    const recipientName = recipientUser?.username || recipientWallet.address;

    // Record on permanent ledger — validates via mempool, updates wallet caches
    // This is the ONLY path for transfers. Nothing bypasses this.
    const epoch = await ledger.getCurrentEpoch();
    const entry = await ledger.recordTransfer(senderName, recipientName, amount, 'BTCPC', null, epoch, memo || null);
    const txHash = require('../chain/blockStore').hashLedgerEntry(entry.toObject());

    // Broadcast to P2P network — all nodes see this immediately
    try {
      const tx = {
        type: 'TRANSFER', from: senderName, to: recipientName,
        amount, token: 'BTCPC', epoch, nonce: Date.now(),
        timestamp: Date.now(), memo: memo || null, txHash
      };
      const txMsg = createTransactionMessage(tx, p2p.NODE_ID);
      p2p.broadcast(txMsg);
    } catch (_) {
      // Non-fatal: tx is on the ledger, will propagate at epoch finalization
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
        timestamp: entry.timestamp
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
    // Phase C: Transaction history still indexes by wallet.address. Keep the
    // Wallet lookup here to get the address — Transaction (legacy ledger
    // cache) remains on Mongo. Phase D will move this to LedgerEntry.find
    // keyed by username.
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

"use strict";
const crypto = require('crypto');
const ledger = require('../services/ledger');
const privateAuthorization = require('../services/privateAuthorization');
const p2p = require('../p2p/network');
const { createTransactionMessage } = require('../p2p/protocol');
const stateStore = require('../chain/stateStore');
const { rejectObjectInputs, sanitizeString, sanitizeAmount, validAddress, validChain } = require('../middlewares/validate');
const { resolveUserFromAuth } = require('../services/userResolver');
const { deriveHoneAddress } = require('../wallet/keyManager');

function _honeAddress(username) {
  const acct = stateStore.getAccount ? stateStore.getAccount(username) : null;
  const ownerKey = acct && acct.public_keys && acct.public_keys.owner;
  if (ownerKey) {
    try { return deriveHoneAddress(ownerKey); } catch (_) {}
  }
  // Legacy fallback for accounts without stored public keys
  return 'HONE' + crypto.createHash('sha256').update(username).digest('hex').slice(0, 40);
}

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

    const user = await resolveUserFromAuth(req.user);
    if (!user) return res.status(404).json({ error: 'User not found' });

    const address = _honeAddress(user.username);

    res.status(201).json({
      success: true,
      wallet: {
        address,
        chain,
        balance: { HONE: stateStore.getBalance(user.username, 'HONE') }
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
    const user = await resolveUserFromAuth(req.user);
    if (!user) return res.status(404).json({ error: 'User not found' });

    const username = user.username;
    const honeBalance = stateStore.getBalance(username, 'HONE');
    const tokenBalances = stateStore.getTokenBalances ? stateStore.getTokenBalances(username) : { HONE: honeBalance };
    if (tokenBalances.HONE === undefined) tokenBalances.HONE = honeBalance;
    const delegatedBalance = stateStore.getDelegatedBalance ? stateStore.getDelegatedBalance(username, 'HONE') : 0;

    const address = _honeAddress(username);

    res.json({
      success: true,
      address,
      chain: 'hone',
      balance: tokenBalances,
      delegated_balance: delegatedBalance,
      delegated_note: delegatedBalance > 0 ? 'Delegated tokens can pay for direct network services, but cannot be transferred, sold, staked, or bridged.' : undefined,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Transfer HONE tokens to another account
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

    const senderUser = await resolveUserFromAuth(req.user);
    if (!senderUser) return res.status(404).json({ error: 'Sender not found' });
    const senderName = senderUser.username;

    // Validate sufficient balance via stateStore (O(1) in-memory)
    const senderBalance = stateStore.getBalance(senderName, 'HONE');
    if (senderBalance < amount) {
      return res.status(400).json({ error: 'Insufficient HONE balance' });
    }

    // Resolve recipient — stateStore is the source of truth (blockchain-only)
    let recipientName = null;
    if (!toAddress.startsWith('HONE')) {
      // Treat as username; confirm account exists on chain
      const acct = stateStore.getAccount ? stateStore.getAccount(toAddress) : null;
      if (acct) recipientName = toAddress;
    }
    if (!recipientName) {
      const aliasTarget = stateStore.resolveAccountName ? stateStore.resolveAccountName(toAddress) : null;
      if (aliasTarget) recipientName = aliasTarget;
    }
    if (!recipientName) {
      // Unknown — allow forward-sending: tokens held at username until they register
      // (HONE account names are reserved on first send, no Mongo lookup required)
      if (!toAddress.startsWith('HONE') && toAddress.length >= 3) {
        recipientName = toAddress; // tokens will land when account registers on chain
      } else {
        return res.status(404).json({ error: 'Recipient not found. Use a username or registered address.' });
      }
    }

    // Prevent self-transfer
    if (senderName === recipientName) {
      return res.status(400).json({ error: 'Cannot transfer to your own wallet' });
    }

    const privateAuth = senderUser.privateAuth && senderUser.privateAuth.enabled ? req.body.private_auth : null;
    let authorization = null;
    if (senderUser.privateAuth && senderUser.privateAuth.enabled) {
      if (!privateAuth) {
        return res.status(403).json({ error: 'Private authorization required for this account' });
      }
      try {
        authorization = await privateAuthorization.verifyTransferAuthorization(
          senderName,
          { from: senderName, to: recipientName, amount, token: 'HONE', memo },
          privateAuth
        );
      } catch (authErr) {
        return res.status(403).json({ error: authErr.message });
      }
    }

    // Record on permanent ledger
    const epoch = await ledger.getCurrentEpoch();
    const entry = authorization
      ? await ledger.recordAuthorizedTransfer(
          senderName,
          recipientName,
          amount,
          'HONE',
          null,
          epoch,
          memo || null,
          {
            signedBy: 'private_auth',
            requestId: authorization.requestId,
            threshold: authorization.threshold,
            approvalCount: authorization.approvalCount,
            factors: authorization.factors
          }
        )
      : await ledger.recordTransfer(senderName, recipientName, amount, 'HONE', null, epoch, memo || null);
    const txHash = require('../chain/blockStore').hashLedgerEntry(entry && entry.toObject ? entry.toObject() : entry);

    // Broadcast to P2P network
    try {
      const tx = {
        type: 'TRANSFER', from: senderName, to: recipientName,
        amount, token: 'HONE', epoch, nonce: Date.now(),
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
    const user = await resolveUserFromAuth(req.user);
    if (!user) return res.status(404).json({ error: 'User not found' });

    // Return recent ledger entries for this user from stateStore
    // Full history requires block replay — stateStore is the live cache
    const balance = stateStore.getBalance(user.username, 'HONE');
    const address = 'HONE' + crypto.createHash('sha256').update(user.username).digest('hex').slice(0, 40);

    res.json({
      success: true,
      address,
      transactions: [],
      note: 'Full history available via block replay. Current balance: ' + balance + ' HONE'
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

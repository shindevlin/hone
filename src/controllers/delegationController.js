"use strict";
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');
const nodeRegistry = require('../chain/nodeRegistry');
const { rejectObjectInputs, sanitizeAmount, sanitizeString } = require('../middlewares/validate');

const UNLOCK_PERIOD_DAYS = 7;

// In-memory undelegation requests (bridging gap until full block replay):
// "delegator|miner" → { amount, requestedAt, availableAt }
const undelegateRequests = new Map();

/**
 * Delegate BTCPC to a miner. Moves tokens from wallet balance to delegation.
 *
 * Phase E: Wallet, Delegation, Transaction, Node Mongoose models removed.
 * Uses stateStore for balance + nodeRegistry for miner verification.
 */
async function delegate(req, res) {
  try {
    const objErr = rejectObjectInputs(req.body, ['amount', 'miner']);
    if (objErr) return res.status(400).json({ error: objErr });
    const amount = sanitizeAmount(req.body.amount);
    const miner = sanitizeString(req.body.miner, 24);
    if (!amount || amount <= 0) {
      return res.status(400).json({ error: 'Amount must be greater than 0' });
    }
    if (!miner) {
      return res.status(400).json({ error: 'Miner account is required' });
    }

    // req.user.username set by auth middleware from secretStore/chain — no Mongo needed
    const delegatorName = req.user.username;
    if (!delegatorName) return res.status(401).json({ error: 'Account not resolved from token' });

    // Resolve miner — stateStore is source of truth
    const minerName = miner.match(/^[0-9a-fA-F]{24}$/) ? null : miner;
    if (!minerName) return res.status(400).json({ error: 'Miner must be a BTCPC account name' });
    if (!nodeRegistry.isRegistered(minerName)) {
      return res.status(400).json({ error: 'Target account is not a registered miner' });
    }

    if (delegatorName === minerName) {
      return res.status(400).json({ error: 'Cannot delegate to yourself' });
    }

    const btcpcBalance = stateStore.getBalance(delegatorName, 'BTCPC');
    if (btcpcBalance < amount) {
      return res.status(400).json({ error: 'Insufficient BTCPC balance' });
    }

    // Record on permanent ledger
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordDelegate(delegatorName, minerName, amount, 'mining', epoch);

    // Get updated delegation from stateStore
    const delegKey = delegatorName + '|' + minerName;
    const existingDel = stateStore.getDelegation ? stateStore.getDelegation(delegKey) : null;
    const newAmount = existingDel ? (existingDel.amount || 0) + amount : amount;

    res.status(200).json({
      success: true,
      delegation: {
        delegator: delegatorName,
        miner: minerName,
        amount: newAmount,
        delegated_at: new Date(),
        status: 'active'
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Undelegate BTCPC — starts 7-day unlock period.
 */
async function undelegate(req, res) {
  try {
    const objErr = rejectObjectInputs(req.body, ['amount', 'miner']);
    if (objErr) return res.status(400).json({ error: objErr });
    const amount = sanitizeAmount(req.body.amount);
    const miner = sanitizeString(req.body.miner, 24);
    if (!amount || amount <= 0) return res.status(400).json({ error: 'Amount must be greater than 0' });
    if (!miner) return res.status(400).json({ error: 'Miner account is required' });

    const delegatorName2 = req.user.username;
    if (!delegatorName2) return res.status(401).json({ error: 'Account not resolved from token' });
    const minerName2 = miner.match(/^[0-9a-fA-F]{24}$/) ? null : miner;
    if (!minerName2) return res.status(400).json({ error: 'Miner must be a BTCPC account name' });

    const delegKey = delegatorName2 + '|' + minerName2;
    const existingDel = stateStore.getDelegation ? stateStore.getDelegation(delegKey) : null;
    if (!existingDel || (existingDel.amount || 0) <= 0) {
      return res.status(404).json({ error: 'No active delegation to this miner' });
    }

    if (amount > existingDel.amount) {
      return res.status(400).json({
        error: `Amount exceeds delegated balance (${existingDel.amount} BTCPC)`
      });
    }

    const now = new Date();
    const unlockDate = new Date(now);
    unlockDate.setDate(unlockDate.getDate() + UNLOCK_PERIOD_DAYS);

    undelegateRequests.set(delegKey, { amount, requestedAt: now, availableAt: unlockDate });

    res.status(200).json({
      success: true,
      undelegation: {
        delegator: delegatorName2,
        miner: minerName2,
        amount,
        undelegate_requested_at: now,
        undelegate_available_at: unlockDate
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Withdraw delegation — after unlock period, return BTCPC to wallet.
 */
async function withdrawDelegation(req, res) {
  try {
    const delegatorName = req.user.username;
    if (!delegatorName) return res.status(401).json({ error: 'Account not resolved from token' });

    const now = new Date();
    let totalWithdrawn = 0;
    const withdrawn = [];
    const pending = [];

    // Check all undelegation requests for this user
    for (const [key, request] of undelegateRequests) {
      if (!key.startsWith(delegatorName + '|')) continue;
      const minerName = key.split('|')[1];

      if (now >= request.availableAt) {
        totalWithdrawn += request.amount;
        const epoch = await ledger.getCurrentEpoch();
        await ledger.recordUndelegate(minerName, delegatorName, request.amount, epoch, 'Delegation withdrawal');
        undelegateRequests.delete(key);
        withdrawn.push({ miner: minerName, amount: request.amount });
      } else {
        const remaining = Math.ceil((request.availableAt - now) / (1000 * 60 * 60 * 24));
        pending.push({ miner: minerName, amount: request.amount, days_remaining: remaining });
      }
    }

    if (withdrawn.length === 0 && pending.length === 0) {
      return res.status(404).json({ error: 'No undelegating delegations found.' });
    }

    res.status(200).json({
      success: true,
      total_withdrawn: totalWithdrawn,
      withdrawn,
      pending,
      wallet_balance: stateStore.getBalance(delegatorName, 'BTCPC')
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Get delegations for the authenticated user (as delegator).
 */
async function getDelegations(req, res) {
  try {
    const username = req.user.username;
    if (!username) return res.status(401).json({ error: 'Account not resolved from token' });

    // Read delegations from stateStore
    const allDelegations = stateStore.getAllDelegations ? stateStore.getAllDelegations() : {};
    const myDelegations = [];

    for (const [key, del] of Object.entries(allDelegations)) {
      if (key.startsWith(username + '|')) {
        myDelegations.push({
          miner: key.split('|')[1],
          amount: del.amount || 0,
          delegated_at: del.epoch,
          status: 'active'
        });
      }
    }

    res.status(200).json({
      success: true,
      delegations: myDelegations,
      total_delegated: myDelegations.reduce((sum, d) => sum + d.amount, 0)
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Get all delegations TO a miner (public endpoint).
 */
async function getMinerDelegations(req, res) {
  try {
    if (typeof req.params.miner !== 'string') return res.status(400).json({ error: 'invalid miner' });
    const miner = req.params.miner.slice(0, 24);

    const minerTarget = miner.match(/^[0-9a-fA-F]{24}$/) ? null : miner;
    if (!minerTarget) return res.status(400).json({ error: 'Miner must be a BTCPC account name' });

    const allDelegations = stateStore.getAllDelegations ? stateStore.getAllDelegations() : {};
    const minerDelegations = [];

    for (const [key, del] of Object.entries(allDelegations)) {
      if (key.endsWith('|' + minerTarget)) {
        minerDelegations.push({
          delegator: key.split('|')[0],
          amount: del.amount || 0,
          delegated_at: del.epoch
        });
      }
    }

    const totalDelegated = minerDelegations.reduce((sum, d) => sum + d.amount, 0);

    res.status(200).json({
      success: true,
      miner: minerTarget,
      total_delegated: totalDelegated,
      delegator_count: minerDelegations.length,
      delegations: minerDelegations
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

module.exports = {
  delegate,
  undelegate,
  withdrawDelegation,
  getDelegations,
  getMinerDelegations
};

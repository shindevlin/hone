"use strict";
const Wallet = require('../models/Wallet');
const Delegation = require('../models/Delegation');
const Transaction = require('../models/Transaction');
const Node = require('../models/Node');
const User = require('../models/User');

const UNLOCK_PERIOD_DAYS = 7;

/**
 * Delegate BTCPC to a miner. Moves tokens from wallet balance to delegation.
 */
async function delegate(req, res) {
  const userId = req.user.id;
  const { amount, miner } = req.body;

  try {
    if (!amount || amount <= 0) {
      return res.status(400).json({ error: 'Amount must be greater than 0' });
    }

    if (!miner) {
      return res.status(400).json({ error: 'Miner account is required' });
    }

    // Resolve miner — accept username or ObjectId
    let minerUser;
    if (miner.match(/^[0-9a-fA-F]{24}$/)) {
      minerUser = await User.findById(miner);
    } else {
      minerUser = await User.findOne({ username: miner });
    }

    if (!minerUser) {
      return res.status(404).json({ error: 'Miner account not found' });
    }

    // Verify miner has a registered node
    const minerNode = await Node.findOne({
      account: minerUser._id,
      status: { $in: ['registered', 'active'] }
    });

    if (!minerNode) {
      return res.status(400).json({
        error: 'Target account is not a registered miner'
      });
    }

    // Cannot delegate to self
    if (String(minerUser._id) === String(userId)) {
      return res.status(400).json({ error: 'Cannot delegate to yourself' });
    }

    const wallet = await Wallet.findOne({ userId });
    if (!wallet) {
      return res.status(404).json({ error: 'Wallet not found' });
    }

    const btcpcBalance = wallet.balance.get('BTCPC') || 0;
    if (btcpcBalance < amount) {
      return res.status(400).json({ error: 'Insufficient BTCPC balance' });
    }

    // Check for existing active delegation to this miner
    let delegation = await Delegation.findOne({
      delegator: userId,
      miner: minerUser._id,
      status: 'active'
    });

    if (delegation) {
      delegation.amount += amount;
      delegation.delegated_at = new Date();
    } else {
      delegation = new Delegation({
        delegator: userId,
        miner: minerUser._id,
        amount,
        delegated_at: new Date(),
        status: 'active'
      });
    }

    // Deduct from wallet balance
    wallet.balance.set('BTCPC', btcpcBalance - amount);
    await wallet.save();
    await delegation.save();

    // Record delegation transaction
    const tx = new Transaction({
      from: wallet.address,
      to: 'delegation_pool',
      amount,
      type: 'delegation',
      memo: 'BTCPC delegation to miner ' + minerUser.username
    });
    await tx.save();

    res.status(200).json({
      success: true,
      delegation: {
        id: delegation._id,
        miner: minerUser.username,
        amount: delegation.amount,
        delegated_at: delegation.delegated_at,
        status: delegation.status
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
  const userId = req.user.id;
  const { amount, miner } = req.body;

  try {
    if (!amount || amount <= 0) {
      return res.status(400).json({ error: 'Amount must be greater than 0' });
    }

    if (!miner) {
      return res.status(400).json({ error: 'Miner account is required' });
    }

    // Resolve miner
    let minerUser;
    if (miner.match(/^[0-9a-fA-F]{24}$/)) {
      minerUser = await User.findById(miner);
    } else {
      minerUser = await User.findOne({ username: miner });
    }

    if (!minerUser) {
      return res.status(404).json({ error: 'Miner account not found' });
    }

    const delegation = await Delegation.findOne({
      delegator: userId,
      miner: minerUser._id,
      status: 'active'
    });

    if (!delegation) {
      return res.status(404).json({
        error: 'No active delegation to this miner'
      });
    }

    if (amount > delegation.amount) {
      return res.status(400).json({
        error: 'Amount exceeds delegated balance (' + delegation.amount + ' BTCPC)'
      });
    }

    const now = new Date();
    const unlockDate = new Date(now);
    unlockDate.setDate(unlockDate.getDate() + UNLOCK_PERIOD_DAYS);

    if (amount < delegation.amount) {
      // Partial undelegation — split into two records
      const remainingAmount = delegation.amount - amount;
      delegation.amount = remainingAmount;
      await delegation.save();

      // Create undelegating record for the portion being withdrawn
      const undelegation = new Delegation({
        delegator: userId,
        miner: minerUser._id,
        amount,
        delegated_at: delegation.delegated_at,
        status: 'undelegating',
        undelegate_requested_at: now,
        undelegate_available_at: unlockDate
      });
      await undelegation.save();

      res.status(200).json({
        success: true,
        undelegation: {
          id: undelegation._id,
          amount,
          undelegate_requested_at: now,
          undelegate_available_at: unlockDate
        },
        remaining_delegation: {
          id: delegation._id,
          amount: remainingAmount,
          status: 'active'
        }
      });
    } else {
      // Full undelegation
      delegation.status = 'undelegating';
      delegation.undelegate_requested_at = now;
      delegation.undelegate_available_at = unlockDate;
      await delegation.save();

      res.status(200).json({
        success: true,
        undelegation: {
          id: delegation._id,
          amount: delegation.amount,
          undelegate_requested_at: now,
          undelegate_available_at: unlockDate
        }
      });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Withdraw delegation — after unlock period, return BTCPC to wallet.
 */
async function withdrawDelegation(req, res) {
  const userId = req.user.id;
  const { delegation_id } = req.body;

  try {
    let query = {
      delegator: userId,
      status: 'undelegating'
    };
    if (delegation_id) {
      query._id = delegation_id;
    }

    const delegations = await Delegation.find(query);
    if (delegations.length === 0) {
      return res.status(404).json({
        error: 'No undelegating delegations found. Request undelegate first.'
      });
    }

    const now = new Date();
    const wallet = await Wallet.findOne({ userId });
    if (!wallet) {
      return res.status(404).json({ error: 'Wallet not found' });
    }

    let totalWithdrawn = 0;
    const withdrawn = [];
    const pending = [];

    for (const d of delegations) {
      if (now >= d.undelegate_available_at) {
        const returnAmount = d.amount;
        totalWithdrawn += returnAmount;

        d.status = 'withdrawn';
        await d.save();

        const tx = new Transaction({
          from: 'delegation_pool',
          to: wallet.address,
          amount: returnAmount,
          type: 'delegation_withdrawal',
          memo: 'BTCPC delegation withdrawal'
        });
        await tx.save();

        withdrawn.push({ id: d._id, amount: returnAmount });
      } else {
        const remaining = Math.ceil(
          (d.undelegate_available_at - now) / (1000 * 60 * 60 * 24)
        );
        pending.push({
          id: d._id,
          amount: d.amount,
          days_remaining: remaining,
          undelegate_available_at: d.undelegate_available_at
        });
      }
    }

    if (totalWithdrawn > 0) {
      const currentBalance = wallet.balance.get('BTCPC') || 0;
      wallet.balance.set('BTCPC', currentBalance + totalWithdrawn);
      await wallet.save();
    }

    res.status(200).json({
      success: true,
      total_withdrawn: totalWithdrawn,
      withdrawn,
      pending,
      wallet_balance: wallet.balance.get('BTCPC')
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Get delegations for the authenticated user (as delegator).
 */
async function getDelegations(req, res) {
  const userId = req.user.id;

  try {
    const delegations = await Delegation.find({
      delegator: userId,
      status: { $in: ['active', 'undelegating'] }
    }).populate('miner', 'username');

    const result = delegations.map(function (d) {
      return {
        id: d._id,
        miner: d.miner ? d.miner.username : String(d.miner),
        amount: d.amount,
        delegated_at: d.delegated_at,
        status: d.status,
        undelegate_requested_at: d.undelegate_requested_at,
        undelegate_available_at: d.undelegate_available_at
      };
    });

    res.status(200).json({
      success: true,
      delegations: result,
      total_delegated: result
        .filter(function (d) { return d.status === 'active'; })
        .reduce(function (sum, d) { return sum + d.amount; }, 0)
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Get all delegations TO a miner (public endpoint).
 */
async function getMinerDelegations(req, res) {
  const { miner } = req.params;

  try {
    let minerUser;
    if (miner.match(/^[0-9a-fA-F]{24}$/)) {
      minerUser = await User.findById(miner);
    } else {
      minerUser = await User.findOne({ username: miner });
    }

    if (!minerUser) {
      return res.status(404).json({ error: 'Miner account not found' });
    }

    const delegations = await Delegation.find({
      miner: minerUser._id,
      status: 'active'
    }).populate('delegator', 'username');

    const totalDelegated = delegations.reduce(function (sum, d) {
      return sum + d.amount;
    }, 0);

    const result = delegations.map(function (d) {
      return {
        delegator: d.delegator ? d.delegator.username : String(d.delegator),
        amount: d.amount,
        delegated_at: d.delegated_at
      };
    });

    res.status(200).json({
      success: true,
      miner: minerUser.username,
      total_delegated: totalDelegated,
      delegator_count: delegations.length,
      delegations: result
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

"use strict";
const User = require('../models/User');
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');
const { sanitizeAmount } = require('../middlewares/validate');

const MINIMUM_STAKE = 1000;
const UNLOCK_PERIOD_DAYS = 7;

// In-memory unstake requests: username → { amount, requestedAt, availableAt }
// Persisted via ledger UNSTAKE_REQUEST entries; stateStore holds the canonical state.
const unstakeRequests = new Map();

/**
 * Stake BTCPC — move tokens from wallet balance to staked balance.
 * Minimum stake: 1000 BTCPC.
 *
 * Phase E: Wallet, StakingPool, Transaction Mongoose models removed.
 * Uses stateStore for balance checks + ledger for permanent writes.
 */
async function stake(req, res) {
  try {
    if (typeof req.body.amount === 'object' && req.body.amount !== null) {
      return res.status(400).json({ error: 'amount must be a number' });
    }
    const amount = sanitizeAmount(req.body.amount);
    if (!amount || amount < MINIMUM_STAKE) {
      return res.status(400).json({ error: `Minimum stake is ${MINIMUM_STAKE} BTCPC` });
    }

    const user = await User.findById(req.user.id);
    if (!user) return res.status(404).json({ error: 'User not found' });
    const username = user.username;

    const btcpcBalance = stateStore.getBalance(username, 'BTCPC');
    if (btcpcBalance < amount) {
      return res.status(400).json({ error: 'Insufficient BTCPC balance' });
    }

    // Record on permanent ledger
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordStake(username, amount, 'mining', epoch);

    // Get updated stake from stateStore
    const stakePool = stateStore.getStakePool ? stateStore.getStakePool(username) : null;
    const newStaked = stakePool ? (stakePool.total_staked || 0) : amount;

    res.status(200).json({
      success: true,
      staking: {
        staked_amount: newStaked,
        staked_at: new Date(),
        status: 'active',
        username
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Request unstake — initiates a 7-day unlock period.
 */
async function unstake(req, res) {
  try {
    const user = await User.findById(req.user.id);
    if (!user) return res.status(404).json({ error: 'User not found' });
    const username = user.username;

    const stakePool = stateStore.getStakePool ? stateStore.getStakePool(username) : null;
    if (!stakePool || (stakePool.total_staked || 0) <= 0) {
      return res.status(404).json({ error: 'No active stake found' });
    }

    const now = new Date();
    const unlockDate = new Date(now);
    unlockDate.setDate(unlockDate.getDate() + UNLOCK_PERIOD_DAYS);

    unstakeRequests.set(username, {
      amount: stakePool.total_staked,
      requestedAt: now,
      availableAt: unlockDate
    });

    res.status(200).json({
      success: true,
      staking: {
        staked_amount: stakePool.total_staked,
        status: 'unstaking',
        unlock_requested_at: now,
        unlock_available_at: unlockDate
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Withdraw stake — after unlock period, move staked BTCPC back to wallet.
 */
async function withdrawStake(req, res) {
  try {
    const user = await User.findById(req.user.id);
    if (!user) return res.status(404).json({ error: 'User not found' });
    const username = user.username;

    const request = unstakeRequests.get(username);
    if (!request) {
      return res.status(404).json({ error: 'No unstaking request found. Request unstake first.' });
    }

    const now = new Date();
    if (now < request.availableAt) {
      const remaining = Math.ceil((request.availableAt - now) / (1000 * 60 * 60 * 24));
      return res.status(400).json({
        error: `Unlock period not complete. ${remaining} day(s) remaining.`,
        unlock_available_at: request.availableAt
      });
    }

    // Record on permanent ledger
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordUnstake(username, request.amount, epoch, 'Stake withdrawal');

    unstakeRequests.delete(username);

    res.status(200).json({
      success: true,
      withdrawn_amount: request.amount,
      wallet_balance: stateStore.getBalance(username, 'BTCPC')
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Get staking info for the authenticated user.
 */
async function getStakingInfo(req, res) {
  try {
    const user = await User.findById(req.user.id);
    if (!user) return res.status(404).json({ error: 'User not found' });
    const username = user.username;

    const stakePool = stateStore.getStakePool ? stateStore.getStakePool(username) : null;
    const unstakeReq = unstakeRequests.get(username);

    if (!stakePool && !unstakeReq) {
      return res.status(200).json({ success: true, staking: null, message: 'No active stake' });
    }

    res.status(200).json({
      success: true,
      staking: {
        staked_amount: stakePool ? (stakePool.total_staked || 0) : 0,
        status: unstakeReq ? 'unstaking' : 'active',
        unlock_requested_at: unstakeReq ? unstakeReq.requestedAt : null,
        unlock_available_at: unstakeReq ? unstakeReq.availableAt : null,
        username
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Get network-wide staking stats (public endpoint).
 */
async function getNetworkStaking(req, res) {
  try {
    const allPools = stateStore.getAllStakePools ? stateStore.getAllStakePools() : {};
    let totalStaked = 0;
    let stakerCount = 0;
    for (const pool of Object.values(allPools)) {
      if (pool.total_staked > 0) {
        totalStaked += pool.total_staked;
        stakerCount++;
      }
    }

    res.status(200).json({
      success: true,
      network: {
        total_staked: totalStaked,
        total_slashed: 0,
        staker_count: stakerCount,
        minimum_stake: MINIMUM_STAKE,
        unlock_period_days: UNLOCK_PERIOD_DAYS
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

module.exports = {
  stake,
  unstake,
  withdrawStake,
  getStakingInfo,
  getNetworkStaking
};

"use strict";
const Wallet = require('../models/Wallet');
const StakingPool = require('../models/StakingPool');
const Transaction = require('../models/Transaction');

const MINIMUM_STAKE = 1000;
const UNLOCK_PERIOD_DAYS = 7;

/**
 * Stake BTCPC — move tokens from wallet balance to staked balance.
 * Minimum stake: 1000 BTCPC.
 */
async function stake(req, res) {
  const userId = req.user.id;
  const { amount } = req.body;

  try {
    if (!amount || amount < MINIMUM_STAKE) {
      return res.status(400).json({
        error: `Minimum stake is ${MINIMUM_STAKE} BTCPC`
      });
    }

    const wallet = await Wallet.findOne({ userId });
    if (!wallet) {
      return res.status(404).json({ error: 'Wallet not found' });
    }

    const btcpcBalance = wallet.balance.get('BTCPC') || 0;
    if (btcpcBalance < amount) {
      return res.status(400).json({ error: 'Insufficient BTCPC balance' });
    }

    // Check for existing active stake
    let stakeRecord = await StakingPool.findOne({
      account: userId,
      status: 'active'
    });

    if (stakeRecord) {
      // Add to existing stake
      stakeRecord.staked_amount += amount;
      stakeRecord.staked_at = new Date();
    } else {
      // Create new stake
      stakeRecord = new StakingPool({
        account: userId,
        wallet_address: wallet.address,
        staked_amount: amount,
        staked_at: new Date(),
        status: 'active'
      });
    }

    // Deduct from wallet balance
    wallet.balance.set('BTCPC', btcpcBalance - amount);
    await wallet.save();
    await stakeRecord.save();

    // Record staking transaction
    const transaction = new Transaction({
      from: wallet.address,
      to: 'staking_pool',
      amount,
      type: 'staking',
      memo: 'BTCPC stake deposit'
    });
    await transaction.save();

    res.status(200).json({
      success: true,
      staking: {
        staked_amount: stakeRecord.staked_amount,
        staked_at: stakeRecord.staked_at,
        status: stakeRecord.status,
        wallet_address: stakeRecord.wallet_address
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Request unstake — initiates a 7-day unlock period.
 * Cannot unstake if actively mining.
 */
async function unstake(req, res) {
  const userId = req.user.id;

  try {
    const stakeRecord = await StakingPool.findOne({
      account: userId,
      status: 'active'
    });

    if (!stakeRecord) {
      return res.status(404).json({ error: 'No active stake found' });
    }

    // Check if user is actively mining (flag on user or wallet)
    const wallet = await Wallet.findOne({ userId });
    if (wallet && wallet.balance.get('is_mining')) {
      return res.status(400).json({
        error: 'Cannot unstake while actively mining. Stop mining first.'
      });
    }

    const now = new Date();
    const unlockDate = new Date(now);
    unlockDate.setDate(unlockDate.getDate() + UNLOCK_PERIOD_DAYS);

    stakeRecord.status = 'unstaking';
    stakeRecord.unlock_requested_at = now;
    stakeRecord.unlock_available_at = unlockDate;
    await stakeRecord.save();

    res.status(200).json({
      success: true,
      staking: {
        staked_amount: stakeRecord.staked_amount,
        status: stakeRecord.status,
        unlock_requested_at: stakeRecord.unlock_requested_at,
        unlock_available_at: stakeRecord.unlock_available_at
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
  const userId = req.user.id;

  try {
    const stakeRecord = await StakingPool.findOne({
      account: userId,
      status: 'unstaking'
    });

    if (!stakeRecord) {
      return res.status(404).json({
        error: 'No unstaking stake found. Request unstake first.'
      });
    }

    const now = new Date();
    if (now < stakeRecord.unlock_available_at) {
      const remaining = Math.ceil(
        (stakeRecord.unlock_available_at - now) / (1000 * 60 * 60 * 24)
      );
      return res.status(400).json({
        error: `Unlock period not complete. ${remaining} day(s) remaining.`,
        unlock_available_at: stakeRecord.unlock_available_at
      });
    }

    const wallet = await Wallet.findOne({ userId });
    if (!wallet) {
      return res.status(404).json({ error: 'Wallet not found' });
    }

    // Return staked amount (minus any slashed amount) to wallet
    const returnAmount = stakeRecord.staked_amount - stakeRecord.slashed_amount;
    const currentBalance = wallet.balance.get('BTCPC') || 0;
    wallet.balance.set('BTCPC', currentBalance + returnAmount);
    await wallet.save();

    stakeRecord.status = 'withdrawn';
    await stakeRecord.save();

    // Record withdrawal transaction
    const transaction = new Transaction({
      from: 'staking_pool',
      to: wallet.address,
      amount: returnAmount,
      type: 'unstaking',
      memo: 'BTCPC stake withdrawal'
    });
    await transaction.save();

    res.status(200).json({
      success: true,
      withdrawn_amount: returnAmount,
      slashed_amount: stakeRecord.slashed_amount,
      wallet_balance: wallet.balance.get('BTCPC')
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Get staking info for the authenticated user.
 */
async function getStakingInfo(req, res) {
  const userId = req.user.id;

  try {
    const stakeRecord = await StakingPool.findOne({
      account: userId,
      status: { $in: ['active', 'unstaking'] }
    });

    if (!stakeRecord) {
      return res.status(200).json({
        success: true,
        staking: null,
        message: 'No active stake'
      });
    }

    res.status(200).json({
      success: true,
      staking: {
        staked_amount: stakeRecord.staked_amount,
        staked_at: stakeRecord.staked_at,
        status: stakeRecord.status,
        unlock_requested_at: stakeRecord.unlock_requested_at,
        unlock_available_at: stakeRecord.unlock_available_at,
        slashed_amount: stakeRecord.slashed_amount,
        wallet_address: stakeRecord.wallet_address
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
    const result = await StakingPool.aggregate([
      { $match: { status: { $in: ['active', 'unstaking'] } } },
      {
        $group: {
          _id: null,
          total_staked: { $sum: '$staked_amount' },
          total_slashed: { $sum: '$slashed_amount' },
          staker_count: { $sum: 1 }
        }
      }
    ]);

    const stats = result[0] || {
      total_staked: 0,
      total_slashed: 0,
      staker_count: 0
    };

    res.status(200).json({
      success: true,
      network: {
        total_staked: stats.total_staked,
        total_slashed: stats.total_slashed,
        staker_count: stats.staker_count,
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

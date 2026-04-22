"use strict";
const crypto = require('crypto');
const secp256k1 = require('secp256k1');
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');
const { sanitizeAmount, sanitizeString } = require('../middlewares/validate');
const { resolveUserFromAuth } = require('../services/userResolver');

const MINIMUM_STAKE = 1000;
const UNLOCK_PERIOD_DAYS = 7;

// In-memory unstake requests: username → { amount, requestedAt, availableAt }
// Persisted via ledger UNSTAKE_REQUEST entries; stateStore holds the canonical state.
const unstakeRequests = new Map();

function activeKeyChallenge(action, username, amount) {
  return ['BTCPC_STAKE', action, username, amount || ''].join(':');
}

function verifyActiveKeySignature(username, action, amount, body) {
  const account = stateStore.getAccount ? stateStore.getAccount(username) : null;
  const activePub = account && account.public_keys && account.public_keys.active;
  if (!activePub) {
    return { ok: false, status: 422, error: 'account has no active key registered on chain' };
  }

  const expectedChallenge = activeKeyChallenge(action, username, amount);
  const challenge = sanitizeString(body.active_challenge, 200);
  const signature = sanitizeString(body.active_signature, 512);

  if (!challenge || !signature) {
    return {
      ok: false,
      status: 403,
      error: 'active key signature required',
      expected_challenge: expectedChallenge,
    };
  }
  if (challenge !== expectedChallenge) {
    return {
      ok: false,
      status: 400,
      error: 'active_challenge mismatch',
      expected_challenge: expectedChallenge,
    };
  }

  try {
    const msg = crypto.createHash('sha256').update(challenge).digest();
    const valid = secp256k1.ecdsaVerify(Buffer.from(signature, 'hex'), msg, Buffer.from(activePub, 'hex'));
    if (!valid) return { ok: false, status: 401, error: 'active key signature invalid' };
    return { ok: true };
  } catch (_) {
    return { ok: false, status: 400, error: 'invalid active key signature format' };
  }
}

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

    const user = await resolveUserFromAuth(req.user);
    if (!user) return res.status(404).json({ error: 'User not found' });
    const username = user.username;

    const authz = verifyActiveKeySignature(username, 'stake', amount, req.body);
    if (!authz.ok) return res.status(authz.status).json(authz);

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
    const user = await resolveUserFromAuth(req.user);
    if (!user) return res.status(404).json({ error: 'User not found' });
    const username = user.username;

    const authz = verifyActiveKeySignature(username, 'unstake', '', req.body || {});
    if (!authz.ok) return res.status(authz.status).json(authz);

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
    const user = await resolveUserFromAuth(req.user);
    if (!user) return res.status(404).json({ error: 'User not found' });
    const username = user.username;

    const authz = verifyActiveKeySignature(username, 'withdraw', '', req.body || {});
    if (!authz.ok) return res.status(authz.status).json(authz);

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
    const user = await resolveUserFromAuth(req.user);
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

// Per-role base requirements — multiplied by a demand factor at query time.
const ROLE_BASE = {
  inference:      { minStake: 1000, unlockDays: 7,  description: 'Run AI inference proofs. Rewards scale with verified compute delivered.' },
  sensor_data:    { minStake: 500,  unlockDays: 7,  description: 'Relay GNSS / environmental sensor readings from hardware nodes.' },
  storage:        { minStake: 2000, unlockDays: 14, description: 'Host files on BTCPC-FS. Paid per byte delivered; slashed for unavailability.' },
  clock:          { minStake: 1500, unlockDays: 7,  description: 'Provide verified timestamps for chain sync. Requires reliable uptime.' },
  verify_node:    { minStake: 5000, unlockDays: 30, description: 'Validate inference proofs and transaction signatures.' },
  review_node:    { minStake: 3000, unlockDays: 30, description: 'Audit flagged work and handle dispute resolution.' },
  human_reviewer: { minStake: 250,  unlockDays: 3,  description: 'Manual quality review — human oracle for subjective assessment.' },
};

async function getStakeRequirements(req, res) {
  try {
    const allPools = stateStore.getAllStakePools ? stateStore.getAllStakePools() : {};
    let stakerCount = 0;
    for (const pool of Object.values(allPools)) {
      if (pool && pool.total_staked > 0) stakerCount++;
    }
    // +5% per 50 active stakers above a baseline of 10
    const demandMultiplier = Math.max(1.0, 1 + Math.max(0, stakerCount - 10) / 50 * 0.05);

    const requirements = {};
    for (const [role, base] of Object.entries(ROLE_BASE)) {
      requirements[role] = {
        ...base,
        minStake: Math.ceil(base.minStake * demandMultiplier),
        demandMultiplier: Math.round(demandMultiplier * 100) / 100,
      };
    }

    res.json({ success: true, requirements, stakerCount, unlockPeriodDays: UNLOCK_PERIOD_DAYS });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

module.exports = {
  stake,
  unstake,
  withdrawStake,
  getStakingInfo,
  getNetworkStaking,
  getStakeRequirements,
  activeKeyChallenge
};

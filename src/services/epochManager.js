"use strict";

const Epoch = require('../models/Epoch');
const Node = require('../models/Node');
const Wallet = require('../models/Wallet');
const Transaction = require('../models/Transaction');
const StakingPool = require('../models/StakingPool');
const Delegation = require('../models/Delegation');
const { getBlockReward, getCurrentPeriod } = require('./emissionSchedule');

/**
 * BTCPC Epoch Manager
 *
 * Manages the 5-minute epoch cycle: creation, commitment collection,
 * consensus determination, and reward distribution.
 */

const EPOCH_DURATION_MS = 5 * 60 * 1000; // 5 minutes
const DIFFICULTY_ADJUSTMENT_INTERVAL = 1000; // every 1000 epochs (~3.5 days)
const MAX_DIFFICULTY_INCREASE = 4.0;
const MAX_DIFFICULTY_DECREASE = 0.25;
const BASELINE_WORK_PER_EPOCH = 100; // baseline expected work per epoch at difficulty 1.0

// Genesis timestamp — set on first epoch creation if not configured
let GENESIS_TIMESTAMP = null;
let epochInterval = null;
let currentDifficulty = 1.0;

/**
 * Get or initialize the genesis timestamp.
 * Uses BTCPC_GENESIS_TIMESTAMP env var, or the timestamp of epoch 0 in the DB,
 * or creates a new genesis timestamp now.
 */
async function getGenesisTimestamp() {
  if (GENESIS_TIMESTAMP) return GENESIS_TIMESTAMP;

  // Check env var first
  if (process.env.BTCPC_GENESIS_TIMESTAMP) {
    GENESIS_TIMESTAMP = parseInt(process.env.BTCPC_GENESIS_TIMESTAMP);
    return GENESIS_TIMESTAMP;
  }

  // Check if epoch 0 exists in DB
  const genesisEpoch = await Epoch.findOne({ epoch_number: 0 });
  if (genesisEpoch) {
    GENESIS_TIMESTAMP = genesisEpoch.started_at.getTime();
    return GENESIS_TIMESTAMP;
  }

  // No genesis exists yet — will be set when first epoch is created
  return null;
}

/**
 * Calculate the current epoch number based on genesis timestamp.
 */
async function getCurrentEpoch() {
  const genesis = await getGenesisTimestamp();
  if (!genesis) return 0;

  const elapsed = Date.now() - genesis;
  return Math.floor(elapsed / EPOCH_DURATION_MS);
}

/**
 * Create a new epoch record in the database.
 */
async function createEpoch(epochNumber) {
  const genesis = await getGenesisTimestamp();
  const startedAt = new Date(genesis + (epochNumber * EPOCH_DURATION_MS));
  const reward = getBlockReward(epochNumber);

  const epoch = new Epoch({
    epoch_number: epochNumber,
    started_at: startedAt,
    block_reward: reward,
    difficulty: currentDifficulty,
    status: 'active'
  });

  await epoch.save();
  console.log(`[BTCPC] Epoch ${epochNumber} started | reward: ${reward} BTCPC | difficulty: ${currentDifficulty}`);
  return epoch;
}

/**
 * Determine the consensus hash from commitments.
 * Majority hash wins. If tie, the hash submitted earliest wins.
 */
function determineConsensusHash(commitments) {
  if (!commitments || commitments.length === 0) return null;
  if (commitments.length === 1) return commitments[0].state_hash;

  // Count occurrences of each hash
  const hashCounts = {};
  const hashFirstSeen = {};
  for (const c of commitments) {
    if (!hashCounts[c.state_hash]) {
      hashCounts[c.state_hash] = 0;
      hashFirstSeen[c.state_hash] = c.submitted_at;
    }
    hashCounts[c.state_hash]++;
    if (c.submitted_at < hashFirstSeen[c.state_hash]) {
      hashFirstSeen[c.state_hash] = c.submitted_at;
    }
  }

  // Find majority hash
  let maxCount = 0;
  let consensusHash = null;
  for (const [hash, count] of Object.entries(hashCounts)) {
    if (count > maxCount || (count === maxCount && hashFirstSeen[hash] < hashFirstSeen[consensusHash])) {
      maxCount = count;
      consensusHash = hash;
    }
  }

  return consensusHash;
}

/**
 * Distribute rewards for a finalized epoch.
 * Rewards are proportional to each node's verified work (inference_count + tx_count).
 * Only nodes whose state_hash matches consensus receive rewards.
 * Delegators earn proportional share based on their delegation relative to
 * the miner's total stake + delegations.
 * Work is adjusted by difficulty: effective_work = raw_work / difficulty.
 */
async function distributeRewards(epoch) {
  if (!epoch.commitments || epoch.commitments.length === 0) return [];
  if (epoch.block_reward <= 0) return [];

  const consensusHash = epoch.consensus_hash;
  const difficulty = epoch.difficulty || 1.0;

  // Filter to honest commitments (matching consensus hash)
  const honestCommitments = epoch.commitments.filter(c => c.state_hash === consensusHash);
  if (honestCommitments.length === 0) return [];

  // Calculate work_value per miner from WorkProofs: tokens × param_billions
  // This is the fair reward basis — bigger models doing more tokens earn more
  const WorkProof = require('../models/WorkProof');
  const minerWorkValues = {};
  let totalWorkValue = 0;

  for (const commitment of honestCommitments) {
    const nodeId = commitment.node_id.toString();
    const node = await Node.findById(commitment.node_id);
    const user = node ? await require('../models/User').findById(node.account) : null;
    const minerName = user ? user.username : nodeId;

    // Sum work_value from all WorkProofs this miner submitted in this epoch
    const proofs = await WorkProof.find({ epoch_number: epoch.epoch_number, node_id: minerName });
    const workValue = proofs.reduce((sum, p) => sum + (p.work_value || 0), 0);

    minerWorkValues[nodeId] = workValue;
    totalWorkValue += workValue;
  }

  const rewards = [];

  for (const commitment of honestCommitments) {
    const nodeId = commitment.node_id.toString();
    const workValue = minerWorkValues[nodeId] || 0;
    let minerReward;

    if (totalWorkValue === 0) {
      // No work reported — split equally among honest nodes
      minerReward = epoch.block_reward / honestCommitments.length;
    } else {
      // Reward proportional to tokens × param_billions
      minerReward = epoch.block_reward * (workValue / totalWorkValue);
    }

    minerReward = parseFloat(minerReward.toFixed(10));
    if (minerReward <= 0) continue;

    // Find the node's account to credit their wallet
    const node = await Node.findById(commitment.node_id);
    if (!node) continue;

    const minerWallet = await Wallet.findOne({ userId: node.account });
    if (!minerWallet) continue;

    // Check for delegations to this miner
    const activeDelegations = await Delegation.find({
      miner: node.account,
      status: 'active'
    });

    const totalDelegated = activeDelegations.reduce((sum, d) => sum + d.amount, 0);

    // Get miner's own stake
    const minerStake = await StakingPool.findOne({
      account: node.account,
      status: 'active'
    });
    const minerStakeAmount = minerStake ? minerStake.staked_amount : 0;
    const totalPool = minerStakeAmount + totalDelegated;

    if (totalDelegated > 0 && totalPool > 0) {
      // Distribute proportionally between miner and delegators
      const minerShare = minerStakeAmount / totalPool;
      const minerAmount = parseFloat((minerReward * minerShare).toFixed(10));

      // Credit miner their share
      if (minerAmount > 0) {
        const currentBalance = minerWallet.balance.get('BTCPC') || 0;
        minerWallet.balance.set('BTCPC', currentBalance + minerAmount);
        await minerWallet.save();

        const tx = new Transaction({
          from: 'BTCPC_NETWORK',
          to: minerWallet.address,
          amount: minerAmount,
          type: 'mining_reward',
          memo: `Epoch ${epoch.epoch_number} mining reward (miner share)`
        });
        await tx.save();
      }

      // Distribute delegator shares
      for (const delegation of activeDelegations) {
        const delegatorShare = delegation.amount / totalPool;
        const delegatorAmount = parseFloat((minerReward * delegatorShare).toFixed(10));
        if (delegatorAmount <= 0) continue;

        const delegatorWallet = await Wallet.findOne({ userId: delegation.delegator });
        if (!delegatorWallet) continue;

        const delegatorBalance = delegatorWallet.balance.get('BTCPC') || 0;
        delegatorWallet.balance.set('BTCPC', delegatorBalance + delegatorAmount);
        await delegatorWallet.save();

        const dtx = new Transaction({
          from: 'BTCPC_NETWORK',
          to: delegatorWallet.address,
          amount: delegatorAmount,
          type: 'delegation_reward',
          memo: `Epoch ${epoch.epoch_number} delegation reward`
        });
        await dtx.save();
      }

      rewards.push({ node_id: commitment.node_id, amount: minerReward });
    } else {
      // No delegations — miner gets full reward
      const currentBalance = minerWallet.balance.get('BTCPC') || 0;
      minerWallet.balance.set('BTCPC', currentBalance + minerReward);
      await minerWallet.save();

      const tx = new Transaction({
        from: 'BTCPC_NETWORK',
        to: minerWallet.address,
        amount: minerReward,
        type: 'mining_reward',
        memo: `Epoch ${epoch.epoch_number} mining reward`
      });
      await tx.save();

      rewards.push({ node_id: commitment.node_id, amount: minerReward });
    }
  }

  return rewards;
}

/**
 * Calculate difficulty adjustment every DIFFICULTY_ADJUSTMENT_INTERVAL epochs.
 * new_difficulty = old_difficulty * (actual_work / target_work)
 * Clamped to max 4x increase or 0.25x decrease per period.
 */
async function adjustDifficulty(epochNumber) {
  if (epochNumber === 0) return currentDifficulty;
  if (epochNumber % DIFFICULTY_ADJUSTMENT_INTERVAL !== 0) return currentDifficulty;

  const startEpoch = epochNumber - DIFFICULTY_ADJUSTMENT_INTERVAL;

  // Sum total work across the last DIFFICULTY_ADJUSTMENT_INTERVAL epochs
  const workAgg = await Epoch.aggregate([
    {
      $match: {
        epoch_number: { $gte: startEpoch, $lt: epochNumber },
        status: 'finalized'
      }
    },
    {
      $group: {
        _id: null,
        actual_work: { $sum: '$total_work' }
      }
    }
  ]);

  const actualWork = workAgg.length > 0 ? workAgg[0].actual_work : 0;
  const targetWork = BASELINE_WORK_PER_EPOCH * DIFFICULTY_ADJUSTMENT_INTERVAL * currentDifficulty;

  if (targetWork === 0 || actualWork === 0) return currentDifficulty;

  let adjustmentRatio = actualWork / targetWork;

  // Clamp to max 4x increase or 0.25x decrease
  if (adjustmentRatio > MAX_DIFFICULTY_INCREASE) {
    adjustmentRatio = MAX_DIFFICULTY_INCREASE;
  } else if (adjustmentRatio < MAX_DIFFICULTY_DECREASE) {
    adjustmentRatio = MAX_DIFFICULTY_DECREASE;
  }

  const newDifficulty = parseFloat((currentDifficulty * adjustmentRatio).toFixed(10));
  currentDifficulty = newDifficulty;

  console.log(`[BTCPC] Difficulty adjusted at epoch ${epochNumber}: ${newDifficulty} (ratio: ${adjustmentRatio.toFixed(4)})`);
  return newDifficulty;
}

/**
 * Get the current network difficulty.
 */
function getDifficulty() {
  return currentDifficulty;
}

/**
 * Get the next difficulty adjustment epoch number.
 */
function getNextAdjustmentEpoch(currentEpochNum) {
  return Math.ceil((currentEpochNum + 1) / DIFFICULTY_ADJUSTMENT_INTERVAL) * DIFFICULTY_ADJUSTMENT_INTERVAL;
}

/**
 * Finalize an epoch: determine consensus, distribute rewards, mark as finalized.
 */
async function finalizeEpoch(epochNumber) {
  const epoch = await Epoch.findOne({ epoch_number: epochNumber });
  if (!epoch) {
    console.error(`[BTCPC] Epoch ${epochNumber} not found for finalization`);
    return null;
  }

  if (epoch.status === 'finalized') {
    return epoch;
  }

  // Determine consensus hash
  epoch.consensus_hash = determineConsensusHash(epoch.commitments);

  // Calculate total work
  epoch.total_work = epoch.commitments.reduce((sum, c) => sum + c.inference_count + c.tx_count, 0);

  // Distribute rewards
  const rewards = await distributeRewards(epoch);
  epoch.rewards_distributed = rewards;

  // Mark finalized
  epoch.ended_at = new Date();
  epoch.status = 'finalized';
  await epoch.save();

  // Check for difficulty adjustment
  await adjustDifficulty(epochNumber + 1);

  console.log(`[BTCPC] Epoch ${epochNumber} finalized | commitments: ${epoch.commitments.length} | reward distributed: ${epoch.block_reward} BTCPC | difficulty: ${currentDifficulty}`);
  return epoch;
}

/**
 * The epoch tick: finalize previous epoch, create new one.
 */
async function epochTick() {
  try {
    const currentEpochNum = await getCurrentEpoch();

    // Check if current epoch already exists
    const existing = await Epoch.findOne({ epoch_number: currentEpochNum });
    if (existing && existing.status === 'active') {
      // Nothing to do yet — epoch still active
      return;
    }

    // Finalize the previous epoch if it exists and is still active
    const prevEpoch = await Epoch.findOne({ epoch_number: currentEpochNum - 1, status: 'active' });
    if (prevEpoch) {
      await finalizeEpoch(currentEpochNum - 1);
    }

    // Create the new epoch if it doesn't exist
    if (!existing) {
      await createEpoch(currentEpochNum);
    }
  } catch (err) {
    console.error('[BTCPC] Epoch tick error:', err.message);
  }
}

/**
 * Start the epoch loop. Creates genesis epoch if needed, then ticks every 5 minutes.
 */
async function startEpochLoop() {
  console.log('[BTCPC] Starting epoch manager...');

  // Initialize difficulty from latest epoch in DB
  const latestEpoch = await Epoch.findOne().sort({ epoch_number: -1 });
  if (latestEpoch && latestEpoch.difficulty) {
    currentDifficulty = latestEpoch.difficulty;
    console.log(`[BTCPC] Difficulty initialized from DB: ${currentDifficulty}`);
  }

  // Initialize genesis if needed
  let genesis = await getGenesisTimestamp();
  if (!genesis) {
    GENESIS_TIMESTAMP = Date.now();
    console.log(`[BTCPC] Genesis timestamp set: ${new Date(GENESIS_TIMESTAMP).toISOString()}`);

    // Create epoch 0 — the genesis epoch
    await createEpoch(0);
  } else {
    // Run an immediate tick to catch up
    await epochTick();
  }

  // Start the interval
  epochInterval = setInterval(epochTick, EPOCH_DURATION_MS);
  console.log(`[BTCPC] Epoch loop running — ${EPOCH_DURATION_MS / 1000}s interval`);
}

/**
 * Stop the epoch loop (for graceful shutdown).
 */
function stopEpochLoop() {
  if (epochInterval) {
    clearInterval(epochInterval);
    epochInterval = null;
    console.log('[BTCPC] Epoch loop stopped');
  }
}

module.exports = {
  EPOCH_DURATION_MS,
  DIFFICULTY_ADJUSTMENT_INTERVAL,
  startEpochLoop,
  stopEpochLoop,
  finalizeEpoch,
  distributeRewards,
  adjustDifficulty,
  getDifficulty,
  getNextAdjustmentEpoch,
  getCurrentEpoch,
  createEpoch,
  epochTick
};

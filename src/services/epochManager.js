"use strict";

const stateStore = require('../chain/stateStore');
const { getBlockReward, getCurrentPeriod } = require('./emissionSchedule');

/**
 * BTCPC Epoch Manager
 *
 * Manages the 5-minute epoch cycle: creation, commitment collection,
 * consensus determination, and reward distribution.
 */

const EPOCH_DURATION_MS = 30 * 1000; // 30 seconds (v3.0)
const DIFFICULTY_ADJUSTMENT_INTERVAL = 1000; // every 1000 epochs (~3.5 days)
const MAX_DIFFICULTY_INCREASE = 4.0;
const MAX_DIFFICULTY_DECREASE = 0.25;
const BASELINE_WORK_PER_EPOCH = 100; // baseline expected work per epoch at difficulty 1.0

// Genesis timestamp — hardcoded. April 14 2026 7:30 AM Mountain Time (04:30 UTC April 13).
// This is the birth of the BTCPC v3.0 chain. All nodes must agree on this value.
// Every epoch number on every node is derived from this single constant.
const GENESIS_TIMESTAMP = 1776193200000; // 2026-04-14T13:30:00.000Z
let epochInterval = null;
let currentDifficulty = 1.0;

async function getGenesisTimestamp() {
  return GENESIS_TIMESTAMP;
}

/**
 * Calculate the current epoch number based on genesis timestamp.
 */
async function getCurrentEpoch() {
  const genesis = await getGenesisTimestamp();
  if (!genesis) return 0;

  const elapsed = Date.now() - genesis;
  const epoch = Math.floor(elapsed / EPOCH_DURATION_MS);
  return epoch < 0 ? 0 : epoch;
}

/**
 * Create a new epoch record in stateStore.
 */
async function createEpoch(epochNumber) {
  const genesis = await getGenesisTimestamp();
  const startedAt = new Date(genesis + (epochNumber * EPOCH_DURATION_MS));
  const reward = getBlockReward(epochNumber);

  stateStore.setEpoch(epochNumber, {
    epoch_number: epochNumber,
    started_at: startedAt,
    block_reward: reward,
    difficulty: currentDifficulty,
    status: 'active'
  });

  console.log(`[BTCPC] Epoch ${epochNumber} started | reward: ${reward} BTCPC | difficulty: ${currentDifficulty}`);
  return stateStore.getEpoch(epochNumber);
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
 * Reads compute proofs from stateStore — no Mongo.
 */
async function distributeRewards(epoch) {
  if (!epoch.commitments || epoch.commitments.length === 0) return [];
  if (epoch.block_reward <= 0) return [];

  const consensusHash = epoch.consensus_hash;

  // Filter to honest commitments (matching consensus hash)
  const honestCommitments = epoch.commitments.filter(c => c.state_hash === consensusHash);
  if (honestCommitments.length === 0) return [];

  const ledger = require('./ledger');

  // Calculate work_value per miner from stateStore compute proofs
  const minerWorkValues = {};
  let totalWorkValue = 0;

  const computeProofs = stateStore.getComputeProofs(epoch.epoch_number);

  for (const commitment of honestCommitments) {
    const minerName = commitment.node_id;
    // Sum work_value from all compute proofs this miner submitted in this epoch
    const minerProofs = computeProofs.filter(p => p.node_id === minerName);
    const workValue = minerProofs.reduce((sum, p) => sum + (p.work_value || 0), 0);
    minerWorkValues[minerName] = workValue;
    totalWorkValue += workValue;
  }

  const rewards = [];
  const currentEpoch = epoch.epoch_number;

  for (const commitment of honestCommitments) {
    const minerName = commitment.node_id;
    const workValue = minerWorkValues[minerName] || 0;
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

    // Check for delegations from stateStore
    const delegations = stateStore.getDelegationsTo ? stateStore.getDelegationsTo(minerName) : [];
    const totalDelegated = delegations.reduce((sum, d) => sum + (d.amount || 0), 0);
    const minerStakeData = stateStore.getStakePool ? stateStore.getStakePool(minerName) : null;
    const minerStakeAmount = minerStakeData ? (minerStakeData.total_staked || 0) : 0;
    const totalPool = minerStakeAmount + totalDelegated;

    if (totalDelegated > 0 && totalPool > 0) {
      // Distribute proportionally between miner and delegators
      const minerShare = minerStakeAmount / totalPool;
      const minerAmount = parseFloat((minerReward * minerShare).toFixed(10));

      if (minerAmount > 0) {
        await ledger.recordMiningReward(minerName, minerAmount, currentEpoch, 'BTCPC', `Epoch ${currentEpoch} mining reward (miner share)`, 'mining');
      }

      for (const delegation of delegations) {
        const delegatorShare = delegation.amount / totalPool;
        const delegatorAmount = parseFloat((minerReward * delegatorShare).toFixed(10));
        if (delegatorAmount <= 0) continue;
        await ledger.recordMiningReward(delegation.from, delegatorAmount, currentEpoch, 'BTCPC', `Epoch ${currentEpoch} delegation reward`, 'mining');
      }

      rewards.push({ node_id: minerName, amount: minerReward });
    } else {
      // No delegations — miner gets full reward
      await ledger.recordMiningReward(minerName, minerReward, currentEpoch, 'BTCPC', `Epoch ${currentEpoch} mining reward`, 'mining');
      rewards.push({ node_id: minerName, amount: minerReward });
    }
  }

  return rewards;
}

/**
 * Calculate difficulty adjustment every DIFFICULTY_ADJUSTMENT_INTERVAL epochs.
 * Uses stateStore epoch metadata — no Mongo aggregation.
 */
async function adjustDifficulty(epochNumber) {
  if (epochNumber === 0) return currentDifficulty;
  if (epochNumber % DIFFICULTY_ADJUSTMENT_INTERVAL !== 0) return currentDifficulty;

  const startEpoch = epochNumber - DIFFICULTY_ADJUSTMENT_INTERVAL;

  // Sum total work across the last DIFFICULTY_ADJUSTMENT_INTERVAL epochs from stateStore
  let actualWork = 0;
  for (let i = startEpoch; i < epochNumber; i++) {
    const ep = stateStore.getEpoch(i);
    if (ep && ep.status === 'finalized') {
      actualWork += ep.total_work || 0;
    }
  }

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
  const epoch = stateStore.getEpoch(epochNumber);
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
  epoch.total_work = (epoch.commitments || []).reduce((sum, c) => sum + (c.inference_count || 0) + (c.tx_count || 0), 0);

  // Distribute rewards
  const rewards = await distributeRewards(epoch);
  epoch.rewards_distributed = rewards;

  // Mark finalized
  epoch.ended_at = new Date();
  epoch.status = 'finalized';
  stateStore.setEpoch(epochNumber, epoch);

  // Check for difficulty adjustment
  await adjustDifficulty(epochNumber + 1);

  console.log(`[BTCPC] Epoch ${epochNumber} finalized | commitments: ${(epoch.commitments || []).length} | reward distributed: ${epoch.block_reward} BTCPC | difficulty: ${currentDifficulty}`);
  return epoch;
}

/**
 * The epoch tick: finalize previous epoch, create new one.
 */
async function epochTick() {
  try {
    const currentEpochNum = await getCurrentEpoch();

    // Check if current epoch already exists
    const existing = stateStore.getEpoch(currentEpochNum);
    if (existing && existing.status === 'active') {
      // Nothing to do yet — epoch still active
      return;
    }

    // Finalize the previous epoch if it exists and is still active
    const prevEpoch = stateStore.getEpoch(currentEpochNum - 1);
    if (prevEpoch && prevEpoch.status === 'active') {
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

  // Initialize difficulty from the latest epoch in stateStore.
  const latestEpoch = stateStore.getLatestEpoch();
  if (latestEpoch && latestEpoch.difficulty) {
    currentDifficulty = latestEpoch.difficulty;
    console.log(`[BTCPC] Difficulty initialized from chain state: ${currentDifficulty}`);
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

"use strict";

const Epoch = require('../models/Epoch');
const Node = require('../models/Node');
const Wallet = require('../models/Wallet');
const Transaction = require('../models/Transaction');
const { getBlockReward, getCurrentPeriod } = require('./emissionSchedule');

/**
 * BTCPC Epoch Manager
 *
 * Manages the 5-minute epoch cycle: creation, commitment collection,
 * consensus determination, and reward distribution.
 */

const EPOCH_DURATION_MS = 5 * 60 * 1000; // 5 minutes

// Genesis timestamp — set on first epoch creation if not configured
let GENESIS_TIMESTAMP = null;
let epochInterval = null;

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
    status: 'active'
  });

  await epoch.save();
  console.log(`[BTCPC] Epoch ${epochNumber} started | reward: ${reward} BTCPC`);
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
 */
async function distributeRewards(epoch) {
  if (!epoch.commitments || epoch.commitments.length === 0) return [];
  if (epoch.block_reward <= 0) return [];

  const consensusHash = epoch.consensus_hash;

  // Filter to honest commitments (matching consensus hash)
  const honestCommitments = epoch.commitments.filter(c => c.state_hash === consensusHash);
  if (honestCommitments.length === 0) return [];

  // Calculate total work from honest nodes
  const totalWork = honestCommitments.reduce((sum, c) => sum + c.inference_count + c.tx_count, 0);

  const rewards = [];

  for (const commitment of honestCommitments) {
    const nodeWork = commitment.inference_count + commitment.tx_count;
    let amount;

    if (totalWork === 0) {
      // No work reported — split equally among honest nodes
      amount = epoch.block_reward / honestCommitments.length;
    } else {
      amount = epoch.block_reward * (nodeWork / totalWork);
    }

    amount = parseFloat(amount.toFixed(8));
    if (amount <= 0) continue;

    // Find the node's account to credit their wallet
    const node = await Node.findById(commitment.node_id);
    if (!node) continue;

    const wallet = await Wallet.findOne({ userId: node.account });
    if (!wallet) continue;

    // Credit BTCPC to wallet
    const currentBalance = wallet.balance.get('BTCPC') || 0;
    wallet.balance.set('BTCPC', currentBalance + amount);
    await wallet.save();

    // Record the mining reward transaction
    const tx = new Transaction({
      from: 'BTCPC_NETWORK',
      to: wallet.address,
      amount,
      type: 'mining_reward',
      memo: `Epoch ${epoch.epoch_number} mining reward`
    });
    await tx.save();

    rewards.push({ node_id: commitment.node_id, amount });
  }

  return rewards;
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

  console.log(`[BTCPC] Epoch ${epochNumber} finalized | commitments: ${epoch.commitments.length} | reward distributed: ${epoch.block_reward} BTCPC`);
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
  startEpochLoop,
  stopEpochLoop,
  finalizeEpoch,
  distributeRewards,
  getCurrentEpoch,
  createEpoch,
  epochTick
};

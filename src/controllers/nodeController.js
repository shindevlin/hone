"use strict";

const Node = require('../models/Node');
const Epoch = require('../models/Epoch');
const StakingPool = require('../models/StakingPool');
const { getCurrentEpoch } = require('../services/epochManager');
const { getBlockReward, getCurrentPeriod, getPeriodTable } = require('../services/emissionSchedule');

const MIN_STAKE = 1000;

/**
 * Register as a BTCPC mining node.
 * Requires authenticated user with >= 1000 BTCPC staked.
 */
async function registerNode(req, res) {
  const userId = req.user.id;
  const { endpoint, models, hardware, hive_account, base_wallet } = req.body;

  try {
    if (!endpoint) {
      return res.status(400).json({ error: 'endpoint is required' });
    }

    if (!models || !Array.isArray(models) || models.length === 0) {
      return res.status(400).json({ error: 'models must be a non-empty array of model names' });
    }

    // Check if user already has a registered node
    const existingNode = await Node.findOne({ account: userId });
    if (existingNode) {
      return res.status(400).json({ error: 'Node already registered for this account' });
    }

    // Check staking — user must have >= 1000 BTCPC staked
    const stake = await StakingPool.findOne({ account: userId, status: 'active' });
    if (!stake || stake.staked_amount < MIN_STAKE) {
      return res.status(400).json({
        error: `Minimum stake of ${MIN_STAKE} BTCPC required to register a node. Current stake: ${stake ? stake.staked_amount : 0}`
      });
    }

    const node = new Node({
      account: userId,
      hive_account: hive_account || null,
      base_wallet: base_wallet || null,
      endpoint,
      models,
      hardware: hardware || {},
      stake_amount: stake.staked_amount,
      status: 'active',
      registered_at: new Date()
    });

    await node.save();

    res.status(201).json({
      success: true,
      node: {
        id: node._id,
        endpoint: node.endpoint,
        models: node.models,
        hardware: node.hardware,
        stake_amount: node.stake_amount,
        status: node.status,
        registered_at: node.registered_at
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Update node configuration (endpoint, models, hardware).
 */
async function updateNode(req, res) {
  const userId = req.user.id;
  const { endpoint, models, hardware, hive_account, base_wallet } = req.body;

  try {
    const node = await Node.findOne({ account: userId });
    if (!node) {
      return res.status(404).json({ error: 'No node registered for this account' });
    }

    if (node.status === 'banned') {
      return res.status(403).json({ error: 'Node is banned and cannot be updated' });
    }

    if (endpoint) node.endpoint = endpoint;
    if (models && Array.isArray(models) && models.length > 0) node.models = models;
    if (hardware) node.hardware = { ...node.hardware, ...hardware };
    if (hive_account !== undefined) node.hive_account = hive_account;
    if (base_wallet !== undefined) node.base_wallet = base_wallet;

    await node.save();

    res.json({
      success: true,
      node: {
        id: node._id,
        endpoint: node.endpoint,
        models: node.models,
        hardware: node.hardware,
        hive_account: node.hive_account,
        base_wallet: node.base_wallet,
        status: node.status
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Deregister node — remove from active mining.
 */
async function deregisterNode(req, res) {
  const userId = req.user.id;

  try {
    const node = await Node.findOne({ account: userId });
    if (!node) {
      return res.status(404).json({ error: 'No node registered for this account' });
    }

    node.status = 'suspended';
    await node.save();

    res.json({ success: true, message: 'Node deregistered from active mining' });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * List all active nodes (public).
 */
async function getNodes(req, res) {
  try {
    const nodes = await Node.find({ status: 'active' })
      .select('endpoint models hardware reputation registered_at hive_account last_epoch_commitment')
      .sort({ registered_at: 1 });

    res.json({
      success: true,
      count: nodes.length,
      nodes
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Submit epoch commitment — node submits its work proof for the current epoch.
 * Payload: { state_hash, tx_count, inference_count }
 */
async function submitEpochCommitment(req, res) {
  const userId = req.user.id;
  const { state_hash, tx_count, inference_count } = req.body;

  try {
    if (!state_hash) {
      return res.status(400).json({ error: 'state_hash is required' });
    }

    // Find the caller's node
    const node = await Node.findOne({ account: userId, status: 'active' });
    if (!node) {
      return res.status(403).json({ error: 'No active node for this account' });
    }

    // Get current epoch
    const currentEpochNum = await getCurrentEpoch();
    const epoch = await Epoch.findOne({ epoch_number: currentEpochNum, status: 'active' });
    if (!epoch) {
      return res.status(404).json({ error: 'No active epoch found' });
    }

    // Check if this node already submitted for this epoch
    const alreadySubmitted = epoch.commitments.some(
      c => c.node_id.toString() === node._id.toString()
    );
    if (alreadySubmitted) {
      return res.status(400).json({ error: 'Commitment already submitted for this epoch' });
    }

    // Add commitment
    epoch.commitments.push({
      node_id: node._id,
      state_hash,
      tx_count: tx_count || 0,
      inference_count: inference_count || 0,
      submitted_at: new Date()
    });

    await epoch.save();

    // Update node's last epoch commitment
    node.last_epoch_commitment = currentEpochNum;
    await node.save();

    res.json({
      success: true,
      epoch: currentEpochNum,
      commitment: {
        state_hash,
        tx_count: tx_count || 0,
        inference_count: inference_count || 0
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Get current epoch info (public).
 */
async function getEpochInfo(req, res) {
  try {
    const currentEpochNum = await getCurrentEpoch();
    const epoch = await Epoch.findOne({ epoch_number: currentEpochNum });
    const period = getCurrentPeriod(currentEpochNum);

    res.json({
      success: true,
      epoch: {
        epoch_number: currentEpochNum,
        block_reward: getBlockReward(currentEpochNum),
        period: period ? period.period : null,
        period_duration_months: period ? period.duration_months : null,
        status: epoch ? epoch.status : 'pending',
        commitments_count: epoch ? epoch.commitments.length : 0,
        started_at: epoch ? epoch.started_at : null
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Get info for a specific epoch by number (public).
 */
async function getEpochByNumber(req, res) {
  try {
    const epochNumber = parseInt(req.params.number);
    if (isNaN(epochNumber) || epochNumber < 0) {
      return res.status(400).json({ error: 'Invalid epoch number' });
    }

    const epoch = await Epoch.findOne({ epoch_number: epochNumber });
    if (!epoch) {
      return res.status(404).json({ error: 'Epoch not found' });
    }

    const period = getCurrentPeriod(epochNumber);

    res.json({
      success: true,
      epoch: {
        epoch_number: epoch.epoch_number,
        started_at: epoch.started_at,
        ended_at: epoch.ended_at,
        block_reward: epoch.block_reward,
        total_work: epoch.total_work,
        commitments_count: epoch.commitments.length,
        consensus_hash: epoch.consensus_hash,
        rewards_distributed: epoch.rewards_distributed,
        status: epoch.status,
        period: period ? period.period : null
      }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

module.exports = {
  registerNode,
  updateNode,
  deregisterNode,
  getNodes,
  submitEpochCommitment,
  getEpochInfo,
  getEpochByNumber
};

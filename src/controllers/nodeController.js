"use strict";

const nodeRegistry = require('../chain/nodeRegistry');
const stateStore = require('../chain/stateStore');
const User = require('../models/User');
const { getCurrentEpoch } = require('../services/epochManager');
const { getBlockReward, getCurrentPeriod, getPeriodTable } = require('../services/emissionSchedule');
const { rejectObjectInputs, sanitizeString, validEndpoint, validModel, validHexString, isPlainString } = require('../middlewares/validate');

const MIN_STAKE = 1000;

/**
 * Register as a BTCPC mining node.
 * Requires authenticated user with >= 1000 BTCPC staked.
 *
 * Phase E: Node, Epoch, StakingPool Mongoose models removed.
 * Uses nodeRegistry + stateStore.
 */
async function registerNode(req, res) {
  try {
    if (typeof req.body.endpoint === 'object') return res.status(400).json({ error: 'endpoint must be a string' });
    if (typeof req.body.hive_account === 'object') return res.status(400).json({ error: 'hive_account must be a string' });
    if (typeof req.body.base_wallet === 'object') return res.status(400).json({ error: 'base_wallet must be a string' });
    const endpoint = sanitizeString(req.body.endpoint, 500);
    const models = req.body.models;
    const hardware = (req.body.hardware && typeof req.body.hardware === 'object' && !Array.isArray(req.body.hardware)) ? req.body.hardware : {};
    const hive_account = sanitizeString(req.body.hive_account, 20) || null;
    const base_wallet = sanitizeString(req.body.base_wallet, 200) || null;
    if (!endpoint) {
      return res.status(400).json({ error: 'endpoint is required' });
    }
    if (!validEndpoint(endpoint)) return res.status(400).json({ error: 'invalid endpoint' });

    if (!models || !Array.isArray(models) || models.length === 0) {
      return res.status(400).json({ error: 'models must be a non-empty array of model names' });
    }
    if (models.length > 50) return res.status(400).json({ error: 'too many models (max 50)' });
    for (var i = 0; i < models.length; i++) {
      if (!validModel(models[i])) return res.status(400).json({ error: 'invalid model name: ' + models[i] });
    }

    // Resolve username
    const user = await User.findById(req.user.id);
    if (!user) return res.status(404).json({ error: 'User not found' });
    const username = user.username;

    // Check if already registered
    if (nodeRegistry.isRegistered(username)) {
      return res.status(400).json({ error: 'Node already registered for this account' });
    }

    // Check staking from stateStore
    const stakePool = stateStore.getStakePool ? stateStore.getStakePool(username) : null;
    const stakedAmount = stakePool ? (stakePool.total_staked || 0) : 0;
    if (stakedAmount < MIN_STAKE) {
      return res.status(400).json({
        error: `Minimum stake of ${MIN_STAKE} BTCPC required to register a node. Current stake: ${stakedAmount}`
      });
    }

    // Register in nodeRegistry
    const currentEpochNum = await getCurrentEpoch();
    nodeRegistry.registerNode(username, 'miner', stakedAmount, endpoint, currentEpochNum, false);

    res.status(201).json({
      success: true,
      node: {
        username,
        endpoint,
        models,
        hardware,
        stake_amount: stakedAmount,
        status: 'active',
        registered_at: new Date()
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
  try {
    if (req.body.endpoint !== undefined && typeof req.body.endpoint === 'object') {
      return res.status(400).json({ error: 'endpoint must be a string' });
    }
    const endpoint = req.body.endpoint ? sanitizeString(req.body.endpoint, 500) : null;
    const models = req.body.models;

    const user = await User.findById(req.user.id);
    if (!user) return res.status(404).json({ error: 'User not found' });
    const username = user.username;

    const nodeEntry = nodeRegistry.getNode(username);
    if (!nodeEntry) {
      return res.status(404).json({ error: 'No node registered for this account' });
    }

    if (endpoint) {
      if (!validEndpoint(endpoint)) return res.status(400).json({ error: 'invalid endpoint' });
      nodeEntry.p2pAddress = endpoint;
    }

    res.json({
      success: true,
      node: {
        username,
        endpoint: nodeEntry.p2pAddress,
        status: 'active'
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
  try {
    const user = await User.findById(req.user.id);
    if (!user) return res.status(404).json({ error: 'User not found' });

    const nodeEntry = nodeRegistry.getNode(user.username);
    if (!nodeEntry) {
      return res.status(404).json({ error: 'No node registered for this account' });
    }

    // Mark as suspended in registry (simple status field)
    nodeEntry.status = 'suspended';

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
    const allNodes = nodeRegistry.getRegisteredNodes();
    res.json({
      success: true,
      count: allNodes.length,
      nodes: allNodes.map(n => {
        const acct = stateStore.getAccount(n.username);
        const p2pAddress = n.p2pAddress || (acct && acct.p2p_address) || null;
        return {
          username: n.username,
          type: n.type,
          stake: n.stake,
          p2pAddress,
          registeredEpoch: n.registeredEpoch,
          permissioned: n.permissioned
        };
      })
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
  try {
    if (typeof req.body.state_hash === 'object') return res.status(400).json({ error: 'state_hash must be a string' });
    const state_hash = sanitizeString(req.body.state_hash, 128);
    const tx_count = Number(req.body.tx_count) || 0;
    const inference_count = Number(req.body.inference_count) || 0;
    if (!state_hash) {
      return res.status(400).json({ error: 'state_hash is required' });
    }
    if (!validHexString(state_hash, 128)) return res.status(400).json({ error: 'state_hash must be a valid hex string' });
    if (tx_count < 0 || !Number.isInteger(tx_count)) return res.status(400).json({ error: 'tx_count must be a non-negative integer' });
    if (inference_count < 0 || !Number.isInteger(inference_count)) return res.status(400).json({ error: 'inference_count must be a non-negative integer' });

    const user = await User.findById(req.user.id);
    if (!user) return res.status(404).json({ error: 'User not found' });

    // Check node is registered
    const nodeEntry = nodeRegistry.getNode(user.username);
    if (!nodeEntry) {
      return res.status(403).json({ error: 'No active node for this account' });
    }

    // Get current epoch from stateStore
    const currentEpochNum = await getCurrentEpoch();
    const epoch = stateStore.getEpoch(currentEpochNum);
    if (!epoch || epoch.status !== 'active') {
      return res.status(404).json({ error: 'No active epoch found' });
    }

    // Check if already submitted
    const commitments = epoch.commitments || [];
    const alreadySubmitted = commitments.some(c => c.node_id === user.username);
    if (alreadySubmitted) {
      return res.status(400).json({ error: 'Commitment already submitted for this epoch' });
    }

    // Add commitment
    commitments.push({
      node_id: user.username,
      state_hash,
      tx_count: tx_count || 0,
      inference_count: inference_count || 0,
      submitted_at: new Date()
    });
    epoch.commitments = commitments;
    stateStore.setEpoch(currentEpochNum, epoch);

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
    const currentEpochNum = stateStore.getChainHeight();
    const epoch = stateStore.getEpoch(currentEpochNum);
    const period = getCurrentPeriod(currentEpochNum);

    res.json({
      success: true,
      epoch: {
        epoch_number: currentEpochNum,
        block_reward: getBlockReward(currentEpochNum),
        period: period ? period.period : null,
        period_duration_months: period ? period.duration_months : null,
        status: epoch ? epoch.status : 'pending',
        commitments_count: epoch ? (epoch.commitments || []).length : 0,
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

    const epoch = stateStore.getEpoch(epochNumber);
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
        commitments_count: (epoch.commitments || []).length,
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

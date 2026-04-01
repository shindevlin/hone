"use strict";

const Node = require('../models/Node');

/**
 * Consensus configuration — determines redundancy level per model
 * based on how many miners serve that model.
 *
 * Genesis phase: N=1 (solo miner, no redundancy)
 * Growth phase:  N=3 (3 miners verify each job)
 *
 * The switch is automatic and per-model.
 */

const MIN_MINERS_FOR_CONSENSUS = 3;

/**
 * Get the required redundancy (N) for a given model.
 * @param {string} model - Model name
 * @returns {Promise<{n: number, phase: string, miners: number}>}
 */
async function getRedundancy(model) {
  const minerCount = await Node.countDocuments({
    status: 'active',
    models: { $regex: new RegExp('^' + model.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')) }
  });

  if (minerCount >= MIN_MINERS_FOR_CONSENSUS) {
    return { n: 3, phase: 'consensus', miners: minerCount };
  }

  return { n: 1, phase: 'genesis', miners: minerCount };
}

/**
 * Get consensus status for all models on the network.
 */
async function getNetworkConsensusStatus() {
  const nodes = await Node.find({ status: 'active' }).select('models').lean();

  const modelCount = new Map();
  for (const node of nodes) {
    for (const model of (node.models || [])) {
      modelCount.set(model, (modelCount.get(model) || 0) + 1);
    }
  }

  const status = [];
  for (const [model, count] of modelCount) {
    status.push({
      model,
      miners: count,
      redundancy: count >= MIN_MINERS_FOR_CONSENSUS ? 3 : 1,
      phase: count >= MIN_MINERS_FOR_CONSENSUS ? 'consensus' : 'genesis'
    });
  }

  return status.sort((a, b) => b.miners - a.miners);
}

module.exports = { getRedundancy, getNetworkConsensusStatus, MIN_MINERS_FOR_CONSENSUS };

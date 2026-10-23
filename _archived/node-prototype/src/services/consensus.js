"use strict";

const nodeRegistry = require('../chain/nodeRegistry');

/**
 * Consensus configuration — determines redundancy level per model
 * based on how many miners serve that model.
 *
 * Genesis phase: N=1 (solo miner, no redundancy)
 * Growth phase:  N=3 (3 miners verify each job)
 *
 * The switch is automatic and per-model.
 *
 * Phase E: Node Mongoose model removed. Uses nodeRegistry for miner counts.
 */

const MIN_MINERS_FOR_CONSENSUS = 3;

/**
 * Get the required redundancy (N) for a given model.
 * @param {string} model - Model name
 * @returns {Promise<{n: number, phase: string, miners: number}>}
 */
async function getRedundancy(model) {
  // Count miners that serve this model from nodeRegistry
  // nodeRegistry doesn't track per-model availability, so we use total miner count
  // as a conservative estimate. Per-model tracking can be added in Phase F.
  const allNodes = nodeRegistry.getRegisteredNodes();
  const minerCount = allNodes.filter(n => n.type === 'miner').length;

  if (minerCount >= MIN_MINERS_FOR_CONSENSUS) {
    return { n: 3, phase: 'consensus', miners: minerCount };
  }

  return { n: 1, phase: 'genesis', miners: minerCount };
}

/**
 * Get consensus status for all models on the network.
 * Returns summary based on total registered miners.
 */
async function getNetworkConsensusStatus() {
  const allNodes = nodeRegistry.getRegisteredNodes();
  const miners = allNodes.filter(n => n.type === 'miner');
  const count = miners.length;

  // Without per-model tracking in nodeRegistry, return a network-level summary
  return [{
    model: 'network',
    miners: count,
    redundancy: count >= MIN_MINERS_FOR_CONSENSUS ? 3 : 1,
    phase: count >= MIN_MINERS_FOR_CONSENSUS ? 'consensus' : 'genesis'
  }];
}

module.exports = { getRedundancy, getNetworkConsensusStatus, MIN_MINERS_FOR_CONSENSUS };

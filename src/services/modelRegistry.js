"use strict";

const Node = require('../models/Node');
const axios = require('axios');

const OLLAMA_URL = process.env.OLLAMA_URL || 'http://100.122.145.60:11434';

/**
 * Network Model Registry
 *
 * Tracks which models are available across all miners.
 * Miners periodically sync their local Ollama models to their Node record.
 * Users query the registry to see what's available network-wide.
 */

// In-memory cache of unmet demand (model → request count)
const unmetDemand = new Map();

/**
 * Sync local Ollama models to this node's DB record.
 * Called on miner startup and periodically.
 */
async function syncLocalModels(nodeId) {
  try {
    const resp = await axios.get(`${OLLAMA_URL}/api/tags`, { timeout: 10000 });
    const models = (resp.data.models || []).map(m => m.name);

    if (nodeId) {
      await Node.findByIdAndUpdate(nodeId, { models });
    }

    return models;
  } catch (err) {
    console.error('[model-registry] Failed to sync local models:', err.message);
    return [];
  }
}

/**
 * Get all models available across the network with miner counts.
 * @returns {Array<{model, miners, totalVram, avgReputation}>}
 */
async function getNetworkModels() {
  const nodes = await Node.find({ status: 'active' })
    .select('models hardware reputation endpoint')
    .lean();

  const modelMap = new Map();

  for (const node of nodes) {
    for (const model of (node.models || [])) {
      if (!modelMap.has(model)) {
        modelMap.set(model, { model, miners: 0, totalVram: 0, reputationSum: 0 });
      }
      const entry = modelMap.get(model);
      entry.miners++;
      entry.totalVram += node.hardware?.vram_gb || 0;
      entry.reputationSum += node.reputation || 0;
    }
  }

  return Array.from(modelMap.values())
    .map(m => ({
      model: m.model,
      miners: m.miners,
      avg_vram_gb: m.miners > 0 ? Math.round(m.totalVram / m.miners) : 0,
      avg_reputation: m.miners > 0 ? Math.round(m.reputationSum / m.miners) : 0
    }))
    .sort((a, b) => b.miners - a.miners);
}

/**
 * Check if a specific model is available on the network.
 * @param {string} model
 * @returns {Object} { available, miners, demand }
 */
async function checkModelAvailability(model) {
  const count = await Node.countDocuments({
    status: 'active',
    models: { $regex: new RegExp('^' + model.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')) }
  });

  return {
    model,
    available: count > 0,
    miners: count,
    demand: unmetDemand.get(model) || 0
  };
}

/**
 * Record unmet demand when a model is requested but no miner has it.
 */
function recordUnmetDemand(model) {
  const current = unmetDemand.get(model) || 0;
  unmetDemand.set(model, current + 1);
}

/**
 * Get models with unmet demand — signals to miners what to download.
 * @returns {Array<{model, requests}>}
 */
function getUnmetDemand() {
  return Array.from(unmetDemand.entries())
    .map(([model, requests]) => ({ model, requests }))
    .sort((a, b) => b.requests - a.requests);
}

/**
 * Clear demand for a model (when a miner picks it up).
 */
function clearDemand(model) {
  unmetDemand.delete(model);
}

module.exports = {
  syncLocalModels,
  getNetworkModels,
  checkModelAvailability,
  recordUnmetDemand,
  getUnmetDemand,
  clearDemand
};

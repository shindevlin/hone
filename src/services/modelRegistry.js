"use strict";

const nodeRegistry = require('../chain/nodeRegistry');
const axios = require('axios');

const OLLAMA_URL = process.env.OLLAMA_URL || 'http://100.122.145.60:11434';

/**
 * Network Model Registry
 *
 * Tracks which models are available across all miners.
 * Miners periodically sync their local Ollama models to their nodeRegistry entry.
 * Users query the registry to see what's available network-wide.
 *
 * Phase E: Node Mongoose model removed. Uses nodeRegistry (in-memory).
 */

// In-memory cache of unmet demand (model → request count)
const unmetDemand = new Map();

// In-memory model cache per miner: username → string[]
const minerModels = new Map();

/**
 * Detect which inference engine is running at the backend URL.
 * Tries Ollama first, then OpenAI-compatible (vLLM, llama.cpp, etc).
 */
async function detectEngine() {
  // Try Ollama native endpoint
  try {
    const resp = await axios.get(`${OLLAMA_URL}/api/version`, { timeout: 5000 });
    if (resp.data?.version) {
      return { engine: 'ollama', version: resp.data.version };
    }
  } catch (_) {}

  // Try OpenAI-compatible /v1/models (vLLM, llama.cpp, LocalAI, etc)
  try {
    const resp = await axios.get(`${OLLAMA_URL}/v1/models`, { timeout: 5000 });
    if (resp.data?.data) {
      const server = resp.headers?.['server'] || '';
      if (server.includes('vllm')) return { engine: 'vllm', version: server };
      if (server.includes('llama')) return { engine: 'llama.cpp', version: server };
      return { engine: 'other', version: 'openai-compatible' };
    }
  } catch (_) {}

  return { engine: null, version: null };
}

/**
 * Sync local models to in-memory cache. nodeId (username) is optional —
 * if provided, updates the minerModels map for network-wide queries.
 */
async function syncLocalModels(nodeId) {
  try {
    let models = [];

    // Try Ollama native API first
    try {
      const resp = await axios.get(`${OLLAMA_URL}/api/tags`, { timeout: 10000 });
      models = (resp.data.models || []).map(m => m.name);
    } catch (_) {
      // Fall back to OpenAI-compatible /v1/models
      try {
        const resp = await axios.get(`${OLLAMA_URL}/v1/models`, { timeout: 10000 });
        models = (resp.data.data || []).map(m => m.id);
      } catch (_) {}
    }

    if (nodeId) {
      minerModels.set(nodeId, models);
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
  const allNodes = nodeRegistry.getRegisteredNodes();
  const modelMap = new Map();

  for (const node of allNodes) {
    const models = minerModels.get(node.username) || [];
    for (const model of models) {
      if (!modelMap.has(model)) {
        modelMap.set(model, { model, miners: 0, totalVram: 0, reputationSum: 0, engines: new Set() });
      }
      const entry = modelMap.get(model);
      entry.miners++;
    }
  }

  return Array.from(modelMap.values())
    .map(m => ({
      model: m.model,
      miners: m.miners,
      engines: Array.from(m.engines),
      avg_vram_gb: 0,
      avg_reputation: 0
    }))
    .sort((a, b) => b.miners - a.miners);
}

/**
 * Check if a specific model is available on the network.
 * @param {string} model
 * @returns {Object} { available, miners, demand }
 */
async function checkModelAvailability(model) {
  let count = 0;
  for (const [, models] of minerModels) {
    if (models.some(m => m.startsWith(model.split(':')[0]))) count++;
  }

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

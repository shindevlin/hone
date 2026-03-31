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
      // Try to identify from headers or response shape
      const server = resp.headers?.['server'] || '';
      if (server.includes('vllm')) return { engine: 'vllm', version: server };
      if (server.includes('llama')) return { engine: 'llama.cpp', version: server };
      return { engine: 'other', version: 'openai-compatible' };
    }
  } catch (_) {}

  return { engine: null, version: null };
}

/**
 * Sync local models and engine info to this node's DB record.
 * Called on miner startup and periodically.
 */
async function syncLocalModels(nodeId) {
  try {
    let models = [];
    let engine = { engine: null, version: null };

    // Try Ollama native API first
    try {
      const resp = await axios.get(`${OLLAMA_URL}/api/tags`, { timeout: 10000 });
      models = (resp.data.models || []).map(m => m.name);
      engine = await detectEngine();
    } catch (_) {
      // Fall back to OpenAI-compatible /v1/models
      try {
        const resp = await axios.get(`${OLLAMA_URL}/v1/models`, { timeout: 10000 });
        models = (resp.data.data || []).map(m => m.id);
        engine = await detectEngine();
      } catch (_) {}
    }

    if (nodeId) {
      const update = { models };
      if (engine.engine) {
        update.inference_engine = engine.engine;
        update.inference_engine_version = engine.version;
      }
      await Node.findByIdAndUpdate(nodeId, update);
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
    .select('models hardware reputation endpoint inference_engine')
    .lean();

  const modelMap = new Map();

  for (const node of nodes) {
    for (const model of (node.models || [])) {
      if (!modelMap.has(model)) {
        modelMap.set(model, { model, miners: 0, totalVram: 0, reputationSum: 0, engines: new Set() });
      }
      const entry = modelMap.get(model);
      entry.miners++;
      entry.totalVram += node.hardware?.vram_gb || 0;
      entry.reputationSum += node.reputation || 0;
      if (node.inference_engine) entry.engines.add(node.inference_engine);
    }
  }

  return Array.from(modelMap.values())
    .map(m => ({
      model: m.model,
      miners: m.miners,
      engines: Array.from(m.engines),
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

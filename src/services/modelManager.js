"use strict";

/**
 * Model Manager — auto-downloads models based on network demand and disk budget.
 *
 * Miners set BTCPC_MAX_MODEL_STORAGE_GB (default 50GB).
 * On each check, pulls the highest-demand model that fits within budget.
 * Prioritizes smaller models first (more variety = more job types = more earnings).
 */

const axios = require('axios');
const { execSync } = require('child_process');

const OLLAMA_URL = process.env.OLLAMA_URL || 'http://localhost:11434';
const MAX_STORAGE_GB = parseFloat(process.env.BTCPC_MAX_MODEL_STORAGE_GB) || 50;
const CHECK_INTERVAL_MS = parseInt(process.env.BTCPC_MODEL_CHECK_MS) || 1800000; // 30 min

let checkTimer = null;

/**
 * Get current disk usage of all local models.
 */
async function getLocalModelUsage() {
  try {
    const res = await axios.get(`${OLLAMA_URL}/api/tags`, { timeout: 5000 });
    const models = res.data?.models || [];
    let totalBytes = 0;
    const installed = [];

    for (const m of models) {
      totalBytes += m.size || 0;
      installed.push({ name: m.name, sizeGB: (m.size || 0) / 1e9 });
    }

    return {
      models: installed,
      totalGB: totalBytes / 1e9,
      remainingGB: MAX_STORAGE_GB - (totalBytes / 1e9),
      count: models.length
    };
  } catch (err) {
    console.error('[model-mgr] Failed to check local models:', err.message);
    return { models: [], totalGB: 0, remainingGB: MAX_STORAGE_GB, count: 0 };
  }
}

/**
 * Get models that the network needs but no miner (or few miners) serve.
 */
async function getWantedModels() {
  try {
    const { getUnmetDemand } = require('./modelRegistry');
    const demand = getUnmetDemand();
    return demand.map(d => ({ model: d.model, requests: d.requests }));
  } catch (_) {
    return [];
  }
}

/**
 * Get the size of a model from the Ollama registry before downloading.
 */
async function getRegistryModelSize(model) {
  try {
    const parts = model.split(':');
    const library = parts[0];
    const tag = parts[1] || 'latest';

    const res = await axios.get(
      `https://registry.ollama.ai/v2/library/${library}/manifests/${tag}`,
      {
        headers: { 'Accept': 'application/vnd.docker.distribution.manifest.v2+json' },
        timeout: 10000
      }
    );

    const layers = res.data?.layers || [];
    const totalBytes = layers.reduce((sum, l) => sum + (l.size || 0), 0);
    return totalBytes / 1e9;
  } catch (_) {
    return null; // unknown size
  }
}

/**
 * Pull a model via Ollama.
 */
async function pullModel(model) {
  try {
    console.log(`[model-mgr] Pulling ${model}...`);
    const res = await axios.post(`${OLLAMA_URL}/api/pull`, {
      name: model, stream: false
    }, { timeout: 600000 }); // 10 min timeout for large models
    console.log(`[model-mgr] Pulled ${model} successfully`);
    return true;
  } catch (err) {
    console.error(`[model-mgr] Failed to pull ${model}: ${err.message}`);
    return false;
  }
}

/**
 * Check demand and auto-pull models that fit within disk budget.
 */
async function checkAndPullModels() {
  const usage = await getLocalModelUsage();
  const installedNames = usage.models.map(m => m.name);

  console.log(`[model-mgr] Disk: ${usage.totalGB.toFixed(1)}/${MAX_STORAGE_GB}GB (${usage.remainingGB.toFixed(1)}GB free), ${usage.count} models`);

  // Get unmet demand
  const wanted = await getWantedModels();
  if (wanted.length === 0) return;

  console.log(`[model-mgr] Unmet demand: ${wanted.map(w => w.model + '(' + w.requests + ')').join(', ')}`);

  // Sort by demand (most requested first), then filter to what fits
  wanted.sort((a, b) => b.requests - a.requests);

  for (const w of wanted) {
    if (installedNames.includes(w.model)) continue; // already have it

    // Check model size before downloading
    const sizeGB = await getRegistryModelSize(w.model);
    if (sizeGB === null) {
      console.log(`[model-mgr] Skipping ${w.model} — unknown size`);
      continue;
    }

    if (sizeGB > usage.remainingGB) {
      console.log(`[model-mgr] Skipping ${w.model} — ${sizeGB.toFixed(1)}GB exceeds remaining ${usage.remainingGB.toFixed(1)}GB`);
      continue;
    }

    // Pull it
    const success = await pullModel(w.model);
    if (success) {
      usage.remainingGB -= sizeGB;
      console.log(`[model-mgr] ${w.model} installed (${sizeGB.toFixed(1)}GB). ${usage.remainingGB.toFixed(1)}GB remaining`);
    }
  }
}

/**
 * Start the model manager loop.
 */
function startModelManager() {
  if (process.env.BTCPC_AUTO_MODELS === 'false') {
    console.log('[model-mgr] Disabled (BTCPC_AUTO_MODELS=false)');
    return;
  }

  console.log(`[model-mgr] Max storage: ${MAX_STORAGE_GB}GB, checking every ${CHECK_INTERVAL_MS / 60000}min`);

  // Check once on startup (delayed 2min to let miner stabilize)
  setTimeout(() => checkAndPullModels(), 120000);

  // Then periodically
  checkTimer = setInterval(() => checkAndPullModels(), CHECK_INTERVAL_MS);
}

function stopModelManager() {
  if (checkTimer) { clearInterval(checkTimer); checkTimer = null; }
}

module.exports = { startModelManager, stopModelManager, checkAndPullModels, getLocalModelUsage };

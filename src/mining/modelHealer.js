"use strict";

/**
 * modelHealer.js — self-healing model resolution for BTCPC miners.
 *
 * Non-technical home users cannot run commands. Every model fail path
 * auto-repairs: pull → iterate locals → pull fallbacks → stay alive.
 *
 * Fallback chain:
 *  1. Try preferredModel → verifyModel() → return if ok
 *  2. Fail → ollama pull preferredModel → re-verify → return if ok
 *  3. Still fail → iterate ollama list → return first verified local
 *  4. None local → pull qwen3:4b → verify → return if ok
 *  5. Still fail → pull llama3.2:1b → return if ok
 *  6. Still fail → pull qwen2.5:0.5b → return if ok
 *  7. All fail → return null (caller broadcasts MINER_IDLE, stays alive)
 *
 * When preferredModel is null → auto-pick the largest verified local model
 * by parameter count (from Ollama /api/show general.parameter_count).
 *
 * Cache: resolved model is cached until resolveWorkingModel is called
 * with forceRefresh=true.
 */

const axios = require('axios');
const { verifyModel } = require('../services/modelVerifier');

const OLLAMA_URL = process.env.OLLAMA_URL || 'http://localhost:11434';

// Fallback models to pull if nothing local works — smallest first to keep
// pull times reasonable for home users.
const FALLBACK_PULL_LIST = [
  'qwen3:4b',
  'llama3.2:1b',
  'qwen2.5:0.5b',
];

// Module-level cache
let _cachedModel = null;
let _cachePopulated = false;

/**
 * Reset the module cache. Used in tests and when an explicit refresh is needed.
 */
function resetCache() {
  _cachedModel = null;
  _cachePopulated = false;
}

/**
 * List all models available locally on this Ollama instance.
 * Returns array of { name, paramCount } sorted by paramCount desc.
 */
async function listLocalModels() {
  try {
    const res = await axios.get(`${OLLAMA_URL}/api/tags`, { timeout: 5000 });
    const models = res.data?.models || [];
    console.log(`[model-healer] Found ${models.length} local model(s) on Ollama`);
    return models.map(m => ({ name: m.name, paramCount: 0 }));
  } catch (err) {
    console.warn(`[model-healer] Could not list local Ollama models: ${err.message}`);
    return [];
  }
}

/**
 * Query Ollama /api/show for a model's parameter count.
 * Returns 0 if unreachable or unavailable.
 */
async function getParamCount(model) {
  try {
    const res = await axios.post(`${OLLAMA_URL}/api/show`, { name: model }, { timeout: 5000 });
    return res.data?.model_info?.['general.parameter_count'] || 0;
  } catch (_) {
    return 0;
  }
}

/**
 * Pull a model from Ollama registry. Streams pull progress to console.
 * Returns true if pull completed without error, false otherwise.
 */
async function pullModel(modelName) {
  console.log(`[model-healer] Pulling ${modelName} from Ollama registry...`);
  try {
    // Use a long timeout — large models can take minutes to download
    await axios.post(
      `${OLLAMA_URL}/api/pull`,
      { name: modelName },
      { timeout: 600000 } // 10 minutes
    );
    console.log(`[model-healer] Pull complete: ${modelName}`);
    return true;
  } catch (err) {
    console.warn(`[model-healer] Pull failed for ${modelName}: ${err.message}`);
    return false;
  }
}

/**
 * Try to verify a model. Returns the model name if verified, null otherwise.
 */
async function tryVerify(modelName) {
  try {
    const result = await verifyModel(modelName);
    if (result.verified) {
      console.log(`[model-healer] Verified: ${modelName}`);
      return modelName;
    }
    console.log(`[model-healer] Verification failed for ${modelName}: ${result.reason}`);
    return null;
  } catch (err) {
    console.warn(`[model-healer] verifyModel threw for ${modelName}: ${err.message}`);
    return null;
  }
}

/**
 * Auto-pick the largest verified local model by parameter count.
 * Returns the model name or null if none verify.
 */
async function pickLargestLocal() {
  const locals = await listLocalModels();
  if (locals.length === 0) return null;

  // Enrich with param counts in parallel
  const enriched = await Promise.all(
    locals.map(async m => ({
      name: m.name,
      paramCount: await getParamCount(m.name),
    }))
  );

  // Sort largest first
  enriched.sort((a, b) => b.paramCount - a.paramCount);

  for (const m of enriched) {
    const verified = await tryVerify(m.name);
    if (verified) {
      console.log(`[model-healer] Auto-selected largest local model: ${m.name} (${m.paramCount} params)`);
      return verified;
    }
  }

  return null;
}

/**
 * Iterate all local models (unsorted) and return the first that verifies.
 */
async function firstVerifiedLocal() {
  const locals = await listLocalModels();
  for (const m of locals) {
    const verified = await tryVerify(m.name);
    if (verified) return verified;
  }
  return null;
}

/**
 * Main export — resolves a working model name using the self-heal fallback chain.
 *
 * @param {string|null} preferredModel - Model requested by BTCPC_MODEL env var.
 *   Pass null to auto-pick the largest verified local model.
 * @param {boolean} [forceRefresh=false] - If true, ignore cache and re-resolve.
 * @returns {Promise<string|null>} Verified model name or null if all fallbacks fail.
 */
async function resolveWorkingModel(preferredModel, forceRefresh = false) {
  // Return cached result unless refresh requested
  if (_cachePopulated && !forceRefresh) {
    return _cachedModel;
  }

  let resolved = null;

  // Special case: no preferred model → auto-pick largest local
  if (!preferredModel) {
    console.log('[model-healer] BTCPC_MODEL unset — auto-picking largest verified local model');
    resolved = await pickLargestLocal();
    if (!resolved) {
      console.warn('[model-healer] No local models verified — attempting to pull qwen3:4b as default');
      for (const fallback of FALLBACK_PULL_LIST) {
        const pulled = await pullModel(fallback);
        if (pulled) {
          resolved = await tryVerify(fallback);
          if (resolved) break;
        }
      }
    }
    _cachedModel = resolved;
    _cachePopulated = true;
    if (!resolved) {
      console.error('[model-healer] All fallbacks exhausted — no working model. MINER_IDLE.');
    }
    return resolved;
  }

  // Step 1: Try preferred model
  console.log(`[model-healer] Checking preferred model: ${preferredModel}`);
  resolved = await tryVerify(preferredModel);
  if (resolved) {
    _cachedModel = resolved;
    _cachePopulated = true;
    return resolved;
  }

  // Step 2: Pull preferred model and re-verify
  console.log(`[model-healer] ${preferredModel} not verified — attempting ollama pull`);
  const pulled = await pullModel(preferredModel);
  if (pulled) {
    resolved = await tryVerify(preferredModel);
    if (resolved) {
      _cachedModel = resolved;
      _cachePopulated = true;
      return resolved;
    }
  }

  // Step 3: Iterate local models, return first verified
  console.log(`[model-healer] Pull failed or still unverified — scanning local models`);
  resolved = await firstVerifiedLocal();
  if (resolved) {
    _cachedModel = resolved;
    _cachePopulated = true;
    return resolved;
  }

  // Steps 4-6: Pull fallback models in order
  console.log(`[model-healer] No local models verified — trying fallback pull list`);
  for (const fallback of FALLBACK_PULL_LIST) {
    // Don't re-pull preferred model (already tried above)
    if (fallback === preferredModel) continue;
    const ok = await pullModel(fallback);
    if (ok) {
      resolved = await tryVerify(fallback);
      if (resolved) {
        _cachedModel = resolved;
        _cachePopulated = true;
        return resolved;
      }
    }
  }

  // Step 7: All fallbacks exhausted
  console.error('[model-healer] All fallbacks exhausted — no working model. MINER_IDLE.');
  _cachedModel = null;
  _cachePopulated = true;
  return null;
}

module.exports = {
  resolveWorkingModel,
  resetCache,
  // Exported for testing
  _internal: {
    listLocalModels,
    getParamCount,
    pullModel,
    tryVerify,
    pickLargestLocal,
    firstVerifiedLocal,
  },
};

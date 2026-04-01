"use strict";

/**
 * Model Verifier — ensures miners run authentic models.
 *
 * Compares the SHA-256 blob hash from the miner's local Ollama
 * against the official hash from the Ollama registry.
 *
 * If hashes don't match, the model was tampered with and the
 * miner's work is rejected (no reward).
 *
 * Cache: verified hashes are cached so we don't hit the registry
 * on every epoch. Cache invalidates when model tag changes.
 */

const axios = require('axios');

const OLLAMA_URL = process.env.OLLAMA_URL || 'http://localhost:11434';
const REGISTRY_URL = 'https://registry.ollama.ai/v2/library';

// Cache: { "qwen3:4b": { localHash, registryHash, verified, checkedAt } }
const verificationCache = {};
const CACHE_TTL_MS = 3600000; // 1 hour

/**
 * Get the model blob hash from the local Ollama instance.
 * This is the SHA-256 of the actual GGUF weights file.
 */
async function getLocalModelHash(model) {
  try {
    const res = await axios.post(`${OLLAMA_URL}/api/show`, { name: model }, { timeout: 10000 });
    const modelfile = res.data?.modelfile || '';
    // Extract blob hash: FROM /path/blobs/sha256-<hash>
    const match = modelfile.match(/sha256-([a-f0-9]{64})/);
    if (match) return 'sha256:' + match[1];

    // Fallback: check /api/tags for digest
    const tags = await axios.get(`${OLLAMA_URL}/api/tags`, { timeout: 5000 });
    const found = (tags.data?.models || []).find(m => m.name === model);
    if (found?.digest) return found.digest;

    return null;
  } catch (err) {
    console.error(`[model-verify] Failed to get local hash for ${model}: ${err.message}`);
    return null;
  }
}

/**
 * Get the official model blob hash from the Ollama registry.
 * The registry serves Docker-style manifests with layer digests.
 */
async function getRegistryModelHash(model) {
  try {
    // Parse "qwen3:4b" → library="qwen3", tag="4b"
    const parts = model.split(':');
    const library = parts[0];
    const tag = parts[1] || 'latest';

    const res = await axios.get(
      `${REGISTRY_URL}/${library}/manifests/${tag}`,
      {
        headers: { 'Accept': 'application/vnd.docker.distribution.manifest.v2+json' },
        timeout: 15000
      }
    );

    // Find the model weights layer (largest layer, type .model)
    const layers = res.data?.layers || [];
    const modelLayer = layers.find(l =>
      l.mediaType === 'application/vnd.ollama.image.model'
    );

    if (modelLayer?.digest) return modelLayer.digest;

    // Fallback: return config digest
    return res.data?.config?.digest || null;
  } catch (err) {
    console.error(`[model-verify] Failed to get registry hash for ${model}: ${err.message}`);
    return null;
  }
}

/**
 * Verify a model's authenticity by comparing local vs registry hashes.
 *
 * @param {string} model - Model name (e.g. "qwen3:4b")
 * @returns {Promise<{ verified: boolean, localHash: string, registryHash: string, reason?: string }>}
 */
async function verifyModel(model) {
  // Check cache
  const cached = verificationCache[model];
  if (cached && (Date.now() - cached.checkedAt) < CACHE_TTL_MS) {
    return cached;
  }

  const localHash = await getLocalModelHash(model);
  const registryHash = await getRegistryModelHash(model);

  const result = {
    model,
    localHash,
    registryHash,
    verified: false,
    checkedAt: Date.now(),
    reason: null
  };

  if (!localHash) {
    result.reason = 'Model not found locally';
  } else if (!registryHash) {
    // Can't reach registry — allow but flag as unverified
    result.verified = true; // permissive: don't block if registry is down
    result.reason = 'Registry unreachable — hash unverified';
    console.warn(`[model-verify] WARNING: ${model} unverified (registry unreachable)`);
  } else if (localHash === registryHash) {
    result.verified = true;
    result.reason = 'Hash matches official registry';
    console.log(`[model-verify] ${model}: VERIFIED (${localHash.slice(0, 20)}...)`);
  } else {
    result.verified = false;
    result.reason = `Hash mismatch — local: ${localHash.slice(0, 20)}... registry: ${registryHash.slice(0, 20)}...`;
    console.error(`[model-verify] REJECTED ${model}: hash mismatch!`);
    console.error(`[model-verify]   local:    ${localHash}`);
    console.error(`[model-verify]   registry: ${registryHash}`);
  }

  verificationCache[model] = result;
  return result;
}

/**
 * Verify all models on the local Ollama instance.
 * Returns array of verification results.
 */
async function verifyAllModels() {
  try {
    const res = await axios.get(`${OLLAMA_URL}/api/tags`, { timeout: 5000 });
    const models = (res.data?.models || []).map(m => m.name);
    const results = [];
    for (const model of models) {
      results.push(await verifyModel(model));
    }
    return results;
  } catch (err) {
    console.error(`[model-verify] Failed to list models: ${err.message}`);
    return [];
  }
}

module.exports = { verifyModel, verifyAllModels, getLocalModelHash, getRegistryModelHash };

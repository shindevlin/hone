"use strict";

const crypto = require('crypto');
const axios = require('axios');

const OLLAMA_URL = process.env.OLLAMA_URL || 'http://100.122.145.60:11434';
const DEFAULT_MODEL = 'qwen3.5:27b';

/**
 * Model weight = verified parameter count in billions from Ollama engine.
 *
 * Work value = tokens_generated × verified_param_billions
 * Reward split = miner_work_value / total_work_value
 *
 * SECURITY: We query Ollama's /api/show for general.parameter_count
 * rather than trusting the model name. A model named "fake:100b" with
 * 1B actual params will be weighted at 1, not 100.
 *
 * Cache is populated on first use and on model sync. Falls back to
 * name parsing only if Ollama is unreachable (logged as warning).
 */
const verifiedParamCache = {};

/**
 * Query Ollama for the real parameter count of a model.
 * Returns param count in billions (e.g. 4.02 for qwen3:4b).
 */
/**
 * Query Ollama for the real parameter count of a model.
 * Returns the raw parameter count (e.g. 4022468096 for qwen3:4b).
 * This is the actual number — no rounding, no trusting the name.
 */
async function verifyModelParams(model) {
  if (verifiedParamCache[model]) return verifiedParamCache[model];

  try {
    const res = await axios.post(`${OLLAMA_URL}/api/show`, { name: model }, { timeout: 5000 });
    const paramCount = res.data?.model_info?.['general.parameter_count'];
    if (paramCount && paramCount > 0) {
      verifiedParamCache[model] = paramCount;
      console.log(`[HONE] Verified model ${model}: ${paramCount} params`);
      return paramCount;
    }
  } catch (_) {
    // Ollama unreachable — fall back to name parsing with warning
  }

  // Fallback: parse from name, convert to raw count (less secure, logged)
  const fromName = parseModelWeightFromName(model);
  const estimated = Math.round(fromName * 1e9);
  console.warn(`[HONE] WARNING: Using unverified param count for ${model}: ${estimated} (Ollama unreachable)`);
  return estimated;
}

/**
 * Parse param count from model name as fallback.
 * Only used when Ollama is unreachable.
 */
function parseModelWeightFromName(model) {
  if (!model) return 1;
  const match = model.match(/(\d+(?:\.\d+)?)b/i);
  if (match) return parseFloat(match[1]);
  const moe = model.match(/(\d+)x(\d+)b/i);
  if (moe) return parseInt(moe[1]) * parseInt(moe[2]);
  return 1;
}

/**
 * Get model weight (raw param count) synchronously from cache.
 * Must call verifyModelParams() first to populate cache.
 * Falls back to name parsing × 1e9 if not cached.
 */
function getModelWeight(model) {
  if (verifiedParamCache[model]) return verifiedParamCache[model];
  return Math.round(parseModelWeightFromName(model) * 1e9);
}

// Legacy — kept for export compatibility
const MODEL_WEIGHTS = {};

/**
 * Diverse prompt pool for synthetic inference work.
 * Covers technical, creative, analytical, and coding domains.
 */
// Metadata tags describing what is being built — inscribed on genesis dreams
const BUILD_METADATA = [
  { project: "hone", tag: "Proof of Compute consensus engine" },
  { project: "hone", tag: "Cross-chain mining reward system" },
  { project: "hone", tag: "BIP-39 wallet with protocol-level 2FA" },
  { project: "hone", tag: "Decentralized inference API" },
  { project: "hone", tag: "Genesis dream NFT and inscription system" },
  { project: "hone", tag: "Encrypted inference protocol" },
  { project: "hone", tag: "Hierarchical key management" },
  { project: "hone", tag: "Block explorer and network dashboard" },
  { project: "hone", tag: "P2P network layer for sovereign chain" },
  { project: "hone", tag: "Commit-reveal verification for AI compute" },
];

function getEpochMetadata(epochNumber) {
  return BUILD_METADATA[epochNumber % BUILD_METADATA.length];
}

const PROMPT_POOL = [
  // Technical
  "Explain the difference between Merkle trees and Patricia tries in blockchain state management. Include time complexity for lookups.",
  "Describe how GPU memory bandwidth affects large language model inference throughput. What are the key bottlenecks?",
  "What is speculative decoding in LLM inference and how does it improve token generation speed?",
  "Explain the consensus mechanism differences between Nakamoto consensus, Tendermint BFT, and HotStuff.",
  "How does KV-cache quantization affect inference quality and memory usage in transformer models?",
  "Describe the architecture of a modern proof-of-stake validator node, including networking, consensus, and storage layers.",
  "What are the tradeoffs between PagedAttention and FlashAttention for serving LLM inference at scale?",
  "Explain how ring signatures provide transaction privacy in Monero compared to zero-knowledge proofs in Zcash.",

  // Creative writing
  "Write a short scene where a sentient AI discovers it is mining cryptocurrency and questions the purpose of its existence.",
  "Compose a haiku about a GPU running inference at 3 AM in an empty server room.",
  "Write a dialogue between Satoshi Nakamoto and Alan Turing discussing proof of work versus proof of compute.",
  "Describe a cyberpunk city where all economic transactions are settled by AI inference rather than traditional consensus.",
  "Write a monologue from the perspective of the last bitcoin miner in the year 2140.",
  "Create a short fable about a network of machines that learned to cooperate by proving their work to each other.",

  // Analysis
  "Analyze the economic implications of doubling halving intervals versus fixed halving intervals in cryptocurrency emission schedules.",
  "Compare the energy efficiency of proof-of-work, proof-of-stake, and proof-of-compute consensus mechanisms.",
  "What are the game-theoretic incentives for honest behavior in a commit-reveal verification scheme for distributed compute?",
  "Evaluate the security tradeoffs of hierarchical key systems (like Hive) compared to single-key systems (like Ethereum).",
  "Discuss the implications of cross-chain mining rewards on token velocity and price discovery across multiple DEXs.",
  "Analyze why fixed total supply with decreasing emission creates different economic dynamics than inflationary token models.",

  // Coding
  "Write a Node.js function that computes a Merkle root from an array of transaction hashes using SHA-256.",
  "Implement a simple rate limiter in JavaScript using the token bucket algorithm with configurable refill rate.",
  "Write a Python function that validates a proof-of-work hash meets a given difficulty target.",
  "Create a Rust function that serializes a blockchain block header into bytes for hashing.",
  "Write a Go function that implements exponential backoff with jitter for retrying failed API calls.",
  "Implement a JavaScript class for managing an epoch-based reward distribution system with proportional payouts.",
  "Write a TypeScript function that generates deterministic key pairs from a master password and account name using PBKDF2.",
  "Create a Node.js stream processor that computes rolling SHA-256 hashes over chunks of inference output.",
];

/**
 * Send a prompt to Ollama and return work proof data.
 * Retries with exponential backoff if Ollama is busy.
 *
 * @param {string} model
 * @param {string} [customPrompt]
 * @param {object} [opts]
 * @param {number} [opts.temperature=0.7]  — use 0 for ensemble (deterministic)
 * @param {number} [opts.maxTokens=512]    — use fixed value for ensemble (equal work)
 */
async function generateWork(model, customPrompt, opts) {
  model = model || DEFAULT_MODEL;
  const prompt = customPrompt || PROMPT_POOL[Math.floor(Math.random() * PROMPT_POOL.length)];
  const promptHash = crypto.createHash('sha256').update(prompt).digest('hex');
  const temperature = (opts && opts.temperature != null) ? opts.temperature : 0.7;
  const maxTokens = (opts && opts.maxTokens != null) ? opts.maxTokens : 512;

  let attempt = 0;
  const maxAttempts = 5;

  while (attempt < maxAttempts) {
    try {
      const response = await axios.post(`${OLLAMA_URL}/api/generate`, {
        model: model,
        prompt: prompt,
        stream: false,
        options: {
          temperature: temperature,
          num_predict: maxTokens
        }
      }, {
        timeout: 120000
      });

      const resultText = response.data.response || '';
      const tokensGenerated = response.data.eval_count || estimateTokens(resultText);
      const resultHash = crypto.createHash('sha256').update(resultText).digest('hex');
      // Use verified param count from Ollama, not model name
      const paramCount = await verifyModelParams(model);

      return {
        prompt_hash: promptHash,
        result_hash: resultHash,
        result_text: resultText,
        tokens_generated: tokensGenerated,
        model: model,
        model_weight_factor: paramCount,
        work_value: tokensGenerated * paramCount
      };
    } catch (err) {
      attempt++;
      if (attempt >= maxAttempts) {
        throw new Error(`Ollama unreachable after ${maxAttempts} attempts: ${err.message}`);
      }
      const backoff = Math.min(1000 * Math.pow(2, attempt), 30000);
      const jitter = Math.floor(Math.random() * 1000);
      console.log(`[HONE] Ollama busy, retrying in ${(backoff + jitter) / 1000}s (attempt ${attempt}/${maxAttempts})`);
      await sleep(backoff + jitter);
    }
  }
}

/**
 * Rough token estimate when eval_count is not available.
 */
function estimateTokens(text) {
  return Math.max(1, Math.ceil(text.length / 4));
}

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

module.exports = {
  getEpochMetadata,
  BUILD_METADATA,
  generateWork,
  getModelWeight,
  verifyModelParams,
  PROMPT_POOL,
  MODEL_WEIGHTS,
  DEFAULT_MODEL,
  OLLAMA_URL
};

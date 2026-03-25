"use strict";

const crypto = require('crypto');
const axios = require('axios');

const OLLAMA_URL = process.env.OLLAMA_URL || 'http://100.122.145.60:11434';
const DEFAULT_MODEL = 'qwen3.5:27b';

/**
 * Model weight factors per whitepaper section 2.5:
 *   1B-7B   = 1.0x
 *   7B-13B  = 2.0x
 *   13B-30B = 4.0x
 *   30B-70B = 8.0x
 *   70B+    = 16.0x
 */
const MODEL_WEIGHTS = {
  'qwen3.5:27b':       4.0,
  'deepseek-r1:8b':    2.0,
  'glm-4.7-flash':     1.0,
  'llama3:8b':         2.0,
  'llama3:70b':        8.0,
  'mixtral:8x7b':      4.0,
  'codellama:34b':     8.0,
  'phi3:14b':          4.0,
  'gemma2:27b':        4.0,
  'qwen2.5:72b':      16.0,
};

function getModelWeight(model) {
  return MODEL_WEIGHTS[model] || 1.0;
}

/**
 * Diverse prompt pool for synthetic inference work.
 * Covers technical, creative, analytical, and coding domains.
 */
// Metadata tags describing what is being built — inscribed on genesis dreams
const BUILD_METADATA = [
  { project: "btcpc", tag: "Proof of Compute consensus engine" },
  { project: "btcpc", tag: "Cross-chain mining reward system" },
  { project: "btcpc", tag: "BIP-39 wallet with protocol-level 2FA" },
  { project: "btcpc", tag: "OpenAI-compatible decentralized inference API" },
  { project: "btcpc", tag: "Genesis dream NFT and inscription system" },
  { project: "btcpc", tag: "Encrypted end-to-end inference protocol" },
  { project: "btcpc", tag: "Hive-style hierarchical key management" },
  { project: "btcpc", tag: "Block explorer and network dashboard" },
  { project: "btcpc", tag: "P2P network layer for sovereign chain" },
  { project: "btcpc", tag: "Commit-reveal verification for AI compute" },
  { project: "nsfwotica", tag: "AI story generation with style training" },
  { project: "bullship", tag: "Crypto trivia game with Hive rewards" },
  { project: "betchu_bot", tag: "P2P sports betting on Base" },
  { project: "ursOS", tag: "Personal AI assistant with MEGA archival" },
  { project: "redaktly", tag: "PII detection and document redaction" },
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
 */
async function generateWork(model, customPrompt) {
  model = model || DEFAULT_MODEL;
  const prompt = customPrompt || PROMPT_POOL[Math.floor(Math.random() * PROMPT_POOL.length)];
  const promptHash = crypto.createHash('sha256').update(prompt).digest('hex');

  let attempt = 0;
  const maxAttempts = 5;

  while (attempt < maxAttempts) {
    try {
      const response = await axios.post(`${OLLAMA_URL}/api/generate`, {
        model: model,
        prompt: prompt,
        stream: false,
        options: {
          temperature: 0.7,
          num_predict: 512
        }
      }, {
        timeout: 120000
      });

      const resultText = response.data.response || '';
      const tokensGenerated = response.data.eval_count || estimateTokens(resultText);
      const resultHash = crypto.createHash('sha256').update(resultText).digest('hex');
      const weightFactor = getModelWeight(model);

      return {
        prompt_hash: promptHash,
        result_hash: resultHash,
        tokens_generated: tokensGenerated,
        model: model,
        model_weight_factor: weightFactor,
        work_value: tokensGenerated * weightFactor
      };
    } catch (err) {
      attempt++;
      if (attempt >= maxAttempts) {
        throw new Error(`Ollama unreachable after ${maxAttempts} attempts: ${err.message}`);
      }
      const backoff = Math.min(1000 * Math.pow(2, attempt), 30000);
      const jitter = Math.floor(Math.random() * 1000);
      console.log(`[BTCPC] Ollama busy, retrying in ${(backoff + jitter) / 1000}s (attempt ${attempt}/${maxAttempts})`);
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
  PROMPT_POOL,
  MODEL_WEIGHTS,
  DEFAULT_MODEL,
  OLLAMA_URL
};

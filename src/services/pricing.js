"use strict";

const Epoch = require('../models/Epoch');
const WorkProof = require('../models/WorkProof');
const { getModelWeight } = require('../mining/workGenerator');

/**
 * Dynamic pricing engine for BTCPC inference.
 *
 * Two factors determine cost:
 * 1. Network load — busier network = higher price
 * 2. Model size — bigger models cost more (weight factor applied)
 *
 * Base rate: 1 BTCPC = BASE_TOKENS_PER_BTCPC tokens (for a 1.0x weight model)
 * A 4.0x model costs 4x more per token than a 1.0x model.
 */

// Base: 1 BTCPC buys this many tokens at normal load (1.0x weight model)
const BASE_TOKENS_PER_BTCPC = parseInt(process.env.BTCPC_BASE_TOKENS) || 1000;

// How many recent epochs to sample for load calculation
const LOAD_WINDOW = parseInt(process.env.BTCPC_LOAD_WINDOW) || 12; // ~1 hour at 5min epochs

// Price floor/ceiling multipliers (prevents runaway pricing)
const MIN_MULTIPLIER = 0.25;
const MAX_MULTIPLIER = 4.0;

// Cache base pricing for 60s
let cachedBasePricing = null;
let cacheExpiry = 0;

/**
 * Calculate current network load factor (0.0 = idle, 1.0+ = busy).
 */
async function getNetworkLoad() {
  const recentEpochs = await Epoch.find({ status: 'finalized' })
    .sort({ epoch_number: -1 })
    .limit(LOAD_WINDOW)
    .lean();

  if (recentEpochs.length === 0) return 0;

  const epochNumbers = recentEpochs.map(e => e.epoch_number);

  const [totalProofs, externalProofs] = await Promise.all([
    WorkProof.countDocuments({ epoch_number: { $in: epochNumbers } }),
    WorkProof.countDocuments({ epoch_number: { $in: epochNumbers }, node_id: 'inference-api' })
  ]);

  if (totalProofs === 0) return 0;
  return externalProofs / totalProofs;
}

/**
 * Load → price multiplier. 1.0 = base rate.
 */
function loadToMultiplier(load) {
  const multiplier = 0.5 + (load * 2);
  return Math.max(MIN_MULTIPLIER, Math.min(MAX_MULTIPLIER, multiplier));
}

/**
 * Get base pricing (network load only, no model factor).
 * Cached for 60s.
 */
async function getBasePricing() {
  const now = Date.now();
  if (cachedBasePricing && now < cacheExpiry) return cachedBasePricing;

  const load = await getNetworkLoad();
  const loadMultiplier = loadToMultiplier(load);

  cachedBasePricing = {
    loadMultiplier: parseFloat(loadMultiplier.toFixed(4)),
    load: parseFloat(load.toFixed(4)),
    baseRate: BASE_TOKENS_PER_BTCPC
  };
  cacheExpiry = now + 60000;

  return cachedBasePricing;
}

/**
 * Get current pricing for a specific model.
 * @param {string} [model] - Model name (default: qwen3.5:27b)
 * @returns {Object} Full pricing breakdown
 */
async function getCurrentPricing(model) {
  const base = await getBasePricing();
  const modelWeight = model ? getModelWeight(model) : 1.0;

  // Total multiplier = network load * model weight
  const totalMultiplier = base.loadMultiplier * modelWeight;
  const tokensPerBtcpc = Math.floor(BASE_TOKENS_PER_BTCPC / totalMultiplier);
  const costPerToken = 1 / tokensPerBtcpc;

  return {
    tokensPerBtcpc,
    costPerToken,
    loadMultiplier: base.loadMultiplier,
    modelWeight,
    totalMultiplier: parseFloat(totalMultiplier.toFixed(4)),
    load: base.load,
    baseRate: BASE_TOKENS_PER_BTCPC,
    model: model || 'default (1.0x)'
  };
}

/**
 * Calculate cost for a given number of tokens on a specific model.
 * @param {number} tokenCount
 * @param {string} [model] - Model name
 * @returns {Promise<{cost: number, pricing: Object}>}
 */
async function calculateCost(tokenCount, model) {
  const pricing = await getCurrentPricing(model);
  const cost = parseFloat((tokenCount * pricing.costPerToken).toFixed(10));
  return { cost, pricing };
}

/**
 * Calculate an automatic bid for an inference request.
 *
 * Early network: block reward is high, miners earn from mining.
 *   → auto-bid is low (near minimum), miners still process because
 *     block reward covers their GPU time.
 *
 * Mature network: block reward drops, miners depend on fees.
 *   → auto-bid rises to fair market rate based on model + load.
 *
 * @param {string} [model] - Model name
 * @param {number} [estimatedTokens] - Expected output tokens (default 512)
 * @returns {Promise<Object>} { bid, breakdown }
 */
async function getAutoBid(model, estimatedTokens) {
  estimatedTokens = estimatedTokens || 512;

  const pricing = await getCurrentPricing(model);
  const { getBlockReward } = require('../mining/workGenerator');
  const Node = require('../models/Node');

  // Current block reward
  const Epoch = require('../models/Epoch');
  const latestEpoch = await Epoch.findOne().sort({ epoch_number: -1 }).lean();
  const epochNumber = latestEpoch ? latestEpoch.epoch_number : 0;
  const blockReward = getBlockReward ? getBlockReward(epochNumber) : 243;

  // Active miners on the network
  const minerCount = Math.max(1, await Node.countDocuments({ status: 'active' }));

  // What each miner earns per epoch from block reward alone
  const rewardPerMiner = blockReward / minerCount;

  // Fair cost for this inference (3x for redundancy)
  const fairCost = estimatedTokens * pricing.costPerToken * 3;

  // How much of the fair cost is already covered by block reward?
  // If reward per miner is high relative to fair cost, bid can be low
  const blockRewardCoverage = Math.min(1.0, rewardPerMiner / (fairCost * 10));

  // Auto-bid: fair cost scaled down by how much block reward covers
  // Early: blockRewardCoverage ≈ 1.0 → bid ≈ fairCost × 0.05 (minimum)
  // Late: blockRewardCoverage ≈ 0.0 → bid ≈ fairCost × 1.0 (full price)
  const bidMultiplier = Math.max(0.05, 1.0 - (blockRewardCoverage * 0.95));
  const bid = parseFloat((fairCost * bidMultiplier).toFixed(10));

  return {
    bid,
    fair_cost: parseFloat(fairCost.toFixed(10)),
    block_reward_coverage: parseFloat(blockRewardCoverage.toFixed(4)),
    bid_multiplier: parseFloat(bidMultiplier.toFixed(4)),
    miners: minerCount,
    reward_per_miner: parseFloat(rewardPerMiner.toFixed(4)),
    model_weight: pricing.modelWeight,
    estimated_tokens: estimatedTokens
  };
}

module.exports = { getCurrentPricing, calculateCost, getNetworkLoad, getAutoBid };

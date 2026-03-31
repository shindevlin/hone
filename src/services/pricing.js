"use strict";

const Epoch = require('../models/Epoch');
const WorkProof = require('../models/WorkProof');

/**
 * Dynamic pricing engine for BTCPC inference.
 *
 * Base rate: 1 BTCPC = BASE_TOKENS_PER_BTCPC tokens
 * Adjusted by network utilization over the last N epochs.
 *
 * When network is idle → price drops (more tokens per BTCPC)
 * When network is busy → price rises (fewer tokens per BTCPC)
 */

// Base: 1 BTCPC buys this many tokens at normal load
const BASE_TOKENS_PER_BTCPC = parseInt(process.env.BTCPC_BASE_TOKENS) || 1000;

// How many recent epochs to sample for load calculation
const LOAD_WINDOW = parseInt(process.env.BTCPC_LOAD_WINDOW) || 12; // ~1 hour at 5min epochs

// Price floor/ceiling multipliers (prevents runaway pricing)
const MIN_MULTIPLIER = 0.25;  // cheapest: 4x more tokens per BTCPC
const MAX_MULTIPLIER = 4.0;   // most expensive: 4x fewer tokens per BTCPC

// Cache pricing for 60s to avoid hitting DB every request
let cachedPricing = null;
let cacheExpiry = 0;

/**
 * Calculate current network load factor (0.0 = idle, 1.0+ = busy).
 * Based on inference work proofs vs mining work proofs in recent epochs.
 */
async function getNetworkLoad() {
  const recentEpochs = await Epoch.find({ status: 'finalized' })
    .sort({ epoch_number: -1 })
    .limit(LOAD_WINDOW)
    .lean();

  if (recentEpochs.length === 0) return 0;

  const epochNumbers = recentEpochs.map(e => e.epoch_number);

  // Count external inference proofs vs total proofs
  const [totalProofs, externalProofs] = await Promise.all([
    WorkProof.countDocuments({ epoch_number: { $in: epochNumbers } }),
    WorkProof.countDocuments({ epoch_number: { $in: epochNumbers }, node_id: 'inference-api' })
  ]);

  if (totalProofs === 0) return 0;

  // Load = ratio of external inference to total capacity
  // More external requests = higher load
  return externalProofs / totalProofs;
}

/**
 * Get the current price multiplier based on network load.
 * Returns a multiplier where 1.0 = base rate.
 *
 * Load 0.0 (idle)     → multiplier 0.5  (cheap, attract usage)
 * Load 0.25 (light)   → multiplier 0.75
 * Load 0.5 (moderate) → multiplier 1.0  (base rate)
 * Load 0.75 (busy)    → multiplier 1.5
 * Load 1.0+ (full)    → multiplier up to MAX
 */
function loadToMultiplier(load) {
  // Sigmoid-ish curve centered at 0.5 load
  const multiplier = 0.5 + (load * 2);
  return Math.max(MIN_MULTIPLIER, Math.min(MAX_MULTIPLIER, multiplier));
}

/**
 * Get current pricing info.
 * @returns {Object} { tokensPerBtcpc, costPerToken, multiplier, load, baseRate }
 */
async function getCurrentPricing() {
  const now = Date.now();
  if (cachedPricing && now < cacheExpiry) return cachedPricing;

  const load = await getNetworkLoad();
  const multiplier = loadToMultiplier(load);
  const tokensPerBtcpc = Math.floor(BASE_TOKENS_PER_BTCPC / multiplier);
  const costPerToken = 1 / tokensPerBtcpc; // in BTCPC

  cachedPricing = {
    tokensPerBtcpc,
    costPerToken,
    multiplier: parseFloat(multiplier.toFixed(4)),
    load: parseFloat(load.toFixed(4)),
    baseRate: BASE_TOKENS_PER_BTCPC
  };
  cacheExpiry = now + 60000; // cache for 60s

  return cachedPricing;
}

/**
 * Calculate cost for a given number of tokens.
 * @param {number} tokenCount
 * @returns {Promise<{cost: number, pricing: Object}>}
 */
async function calculateCost(tokenCount) {
  const pricing = await getCurrentPricing();
  const cost = parseFloat((tokenCount * pricing.costPerToken).toFixed(8));
  return { cost, pricing };
}

module.exports = { getCurrentPricing, calculateCost, getNetworkLoad };

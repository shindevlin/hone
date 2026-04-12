"use strict";

/**
 * BTCPC Anchor Submission — v2.16-alpha
 * Shin Devlin
 *
 * Demand-driven anchor submission for the four-tier finality model.
 * On-chain state-hash anchors are submitted to external chains by
 * permissionless anchor submitters who earn a fee for doing so.
 *
 * Tier intervals (from memory/btcpc_finality_tiers.md):
 *   Tier 1 — Native finality:    every 1 epoch    (~5 min)  [not anchored externally]
 *   Tier 2 — L2 checkpoint:      every 100 epochs (~8 hours)
 *   Tier 3 — Mainnet anchor:     every 1000 epochs (~3.5 days)
 *   Tier 4 — Bitcoin Deep Seal:  every 10000 epochs (~35 days)
 *
 * Design principles (locked in):
 *   - Demand-driven: anyone can submit an anchor and claim the reward.
 *     No designated submitter. Bridge fees accumulate in anchor_reserve.
 *   - Merkle batching: each write commits a Merkle root of N recent epoch
 *     hashes. Bridges verify specific epochs via off-chain Merkle proofs.
 *   - Anchoring is ADDITIVE, never required. BTCPC consensus continues
 *     without any anchor chain. External anchors provide verifiability only.
 *   - Anchor records are append-only: once submitted, immutable.
 *
 * User-facing tier names:
 *   Tier 2 = "L2 checkpoint"
 *   Tier 3 = "Mainnet anchor" / "anchored to Ethereum"
 *   Tier 4 = "Deep Seal" / "sealed in Bitcoin"
 */

// tier → array of anchor records (append-only)
var anchorHistory = new Map([
  ["l2", []],
  ["ethereum", []],
  ["bitcoin", []],
]);

// Estimated gas costs in USD by tier (2025 estimates with optimizations)
var ANCHOR_COSTS_USD = {
  l2: 0.01,        // Base + Arbitrum via blob/calldata, demand-driven keeps it near-free
  ethereum: 0.05,  // EIP-4844 blob batching; ~$12/year baseline = ~$0.05/anchor
  bitcoin: 1.00,   // OP_RETURN; ~$10/year for 10 seals = ~$1.00/anchor
};

// Anchor rewards in BTCPC by tier (submitter receives this for posting an anchor)
var ANCHOR_REWARDS_BTCPC = {
  l2: 0.01,
  ethereum: 0.10,
  bitcoin: 1.00,
};

// Anchor intervals (epochs)
var ANCHOR_INTERVALS = {
  l2: 100,
  ethereum: 1000,
  bitcoin: 10000,
};

var VALID_TIERS = ["l2", "ethereum", "bitcoin"];

function _round(n) {
  return parseFloat(Number(n).toFixed(10));
}

function _now() {
  return Date.now();
}

function _validateTier(tier) {
  if (VALID_TIERS.indexOf(tier) < 0) {
    throw new Error(
      "invalid tier: " + tier + ". Must be one of: " + VALID_TIERS.join(", ")
    );
  }
}

function _validateEpoch(epochNumber) {
  var n = Number(epochNumber);
  if (!Number.isFinite(n) || n < 0 || Math.floor(n) !== n) {
    throw new Error("epochNumber must be a non-negative integer");
  }
  return n;
}

/**
 * Determine whether a given epoch is an anchor point for the given tier.
 *
 * An epoch is an anchor point if epochNumber > 0 and
 * epochNumber % interval === 0.
 *
 * @param {number} epochNumber
 * @param {string} tier — "l2" | "ethereum" | "bitcoin"
 * @returns {boolean}
 */
function shouldAnchor(epochNumber, tier) {
  _validateTier(tier);
  var n = _validateEpoch(epochNumber);
  if (n === 0) return false;
  var interval = ANCHOR_INTERVALS[tier];
  return n % interval === 0;
}

/**
 * Compute the anchor payload for a given epoch.
 * In production this would hash actual block data from blockStore/stateStore.
 * Here it returns a deterministic placeholder that exercises the interface.
 *
 * @param {number} epochNumber
 * @returns {{ state_root, block_hash, epoch, timestamp, batch_size }}
 */
function computeAnchorPayload(epochNumber) {
  var n = _validateEpoch(epochNumber);

  // Deterministic placeholder hashes based on epoch number
  var epoch32 = n.toString(16).padStart(8, "0");
  var stateRoot = "stateroot_epoch_" + epoch32 + "0".repeat(48 - epoch32.length);
  var blockHash = "blockhash_epoch_" + epoch32 + "0".repeat(48 - epoch32.length);

  return {
    state_root: stateRoot,
    block_hash: blockHash,
    epoch: n,
    timestamp: _now(),
    // Batch covers up to 100 epochs in a single Merkle root for gas efficiency
    batch_size: Math.min(100, n > 0 ? 100 : 1),
  };
}

/**
 * Record that a permissionless anchor submitter posted an anchor
 * to an external chain. Append-only — existing anchors are immutable.
 *
 * @param {string} submitter — BTCPC account name of the submitter
 * @param {string} tier — "l2" | "ethereum" | "bitcoin"
 * @param {number} epochNumber
 * @param {string} txHash — on-chain transaction hash
 * @param {number} [cost] — actual cost in USD (optional, for analytics)
 * @returns {{ submitter, tier, epoch, tx_hash, cost_usd, reward_btcpc, submitted_at }}
 */
function recordAnchorSubmission(submitter, tier, epochNumber, txHash, cost) {
  if (typeof submitter !== "string" || submitter.length === 0) {
    throw new Error("submitter required");
  }
  _validateTier(tier);
  var n = _validateEpoch(epochNumber);
  if (typeof txHash !== "string" || txHash.length === 0) {
    throw new Error("txHash required");
  }

  var history = anchorHistory.get(tier);

  // Check for duplicate epoch anchor on this tier (append-only, idempotent)
  for (var i = 0; i < history.length; i++) {
    if (history[i].epoch === n) {
      throw new Error(
        "anchor for epoch " + n + " on tier " + tier + " already exists — anchors are append-only"
      );
    }
  }

  var record = {
    submitter: submitter,
    tier: tier,
    epoch: n,
    tx_hash: txHash,
    cost_usd: typeof cost === "number" && Number.isFinite(cost) ? cost : ANCHOR_COSTS_USD[tier],
    reward_btcpc: ANCHOR_REWARDS_BTCPC[tier],
    submitted_at: _now(),
  };

  history.push(record);
  return Object.assign({}, record);
}

/**
 * Return recent anchors for a tier, newest first.
 *
 * @param {string} tier
 * @param {number} [limit=10]
 * @returns {Array<object>}
 */
function getAnchorHistory(tier, limit) {
  _validateTier(tier);
  var history = anchorHistory.get(tier);
  var n = typeof limit === "number" && Number.isFinite(limit) && limit > 0 ? limit : 10;
  // Return a shallow copy, newest first
  return history.slice().reverse().slice(0, n).map(function (r) {
    return Object.assign({}, r);
  });
}

/**
 * Return the estimated anchor cost in USD for a given tier.
 *
 * @param {string} tier
 * @returns {number} estimated cost in USD
 */
function estimateAnchorCost(tier) {
  _validateTier(tier);
  return ANCHOR_COSTS_USD[tier];
}

/**
 * Return the BTCPC reward for submitting an anchor on a given tier.
 *
 * @param {string} tier
 * @returns {number} reward in BTCPC
 */
function getAnchorReward(tier) {
  _validateTier(tier);
  return ANCHOR_REWARDS_BTCPC[tier];
}

/**
 * Get the anchor interval (in epochs) for a given tier.
 *
 * @param {string} tier
 * @returns {number}
 */
function getAnchorInterval(tier) {
  _validateTier(tier);
  return ANCHOR_INTERVALS[tier];
}

/**
 * Wipe all anchor history. For tests only.
 */
function resetForTests() {
  anchorHistory.set("l2", []);
  anchorHistory.set("ethereum", []);
  anchorHistory.set("bitcoin", []);
}

module.exports = {
  // Constants
  ANCHOR_INTERVALS: ANCHOR_INTERVALS,
  ANCHOR_COSTS_USD: ANCHOR_COSTS_USD,
  ANCHOR_REWARDS_BTCPC: ANCHOR_REWARDS_BTCPC,
  VALID_TIERS: VALID_TIERS,

  // Core API
  shouldAnchor: shouldAnchor,
  computeAnchorPayload: computeAnchorPayload,
  recordAnchorSubmission: recordAnchorSubmission,
  getAnchorHistory: getAnchorHistory,
  estimateAnchorCost: estimateAnchorCost,
  getAnchorReward: getAnchorReward,
  getAnchorInterval: getAnchorInterval,

  // Tests
  resetForTests: resetForTests,
};

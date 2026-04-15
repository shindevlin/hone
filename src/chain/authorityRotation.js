"use strict";

/**
 * BTCPC Epoch Consensus
 * Shin Devlin
 *
 * Decentralized epoch timing — no single authority.
 * Every registered node independently computes epoch boundaries from genesis time.
 * The network accepts the first valid EPOCH_START from any registered node.
 * Finalization is done by whichever node has collected the most mining proofs.
 *
 * Permission tiers:
 *   permissioned — approved by genesis operator, can start/finalize epochs immediately
 *   permissionless — must stake BTCPC to participate in epoch consensus
 *
 * Consensus rules:
 *   1. Any registered node can broadcast EPOCH_START when the clock says it's time
 *   2. First valid EPOCH_START wins (prevents spam by checking node registration)
 *   3. After EPOCH_END, any registered node can finalize if it has mining proofs
 *   4. Multiple finalization attempts → nodes accept the one with the most work
 *   5. Genesis miner is always permissioned (bootstrap)
 */

var crypto = require("crypto");

var GENESIS_MINER = "shindevlin";

/**
 * Check if a node is eligible to participate in epoch consensus.
 *
 * @param {string} username
 * @param {object} nodeInfo — from nodeRegistry.getNode()
 * @param {number} minStake — minimum stake for permissionless nodes
 * @returns {{ eligible: boolean, reason: string|null }}
 */
function isEpochEligible(username, nodeInfo, minStake) {
  // Genesis miner is always eligible
  if (username === GENESIS_MINER) {
    return { eligible: true, reason: null };
  }

  if (!nodeInfo) {
    return { eligible: false, reason: "not registered" };
  }

  // Permissioned nodes are always eligible
  if (nodeInfo.permissioned) {
    return { eligible: true, reason: null };
  }

  // Permissionless nodes need minimum stake
  if (nodeInfo.stake < (minStake || 100)) {
    return { eligible: false, reason: "insufficient stake (need " + (minStake || 100) + ", have " + nodeInfo.stake + ")" };
  }

  return { eligible: true, reason: null };
}

/**
 * Compute the epoch number from genesis time and current time.
 * Every node computes this independently — consensus of time.
 *
 * @param {number} genesisTimestamp — ms since unix epoch
 * @param {number} epochDurationMs — epoch length in ms (v3.0 default = 30000 = 30 sec)
 * @param {number} [now] — current time (for testing)
 * @returns {number} epoch number
 */
function computeEpochNumber(genesisTimestamp, epochDurationMs, now) {
  var t = (now || Date.now()) - genesisTimestamp;
  if (t < 0) return 0;
  return Math.floor(t / epochDurationMs);
}

/**
 * Compute when a specific epoch starts (absolute timestamp).
 */
function epochStartTime(epochNumber, genesisTimestamp, epochDurationMs) {
  return genesisTimestamp + (epochNumber * epochDurationMs);
}

/**
 * Determine if it's time to finalize an epoch.
 * An epoch should be finalized when the next epoch boundary has passed.
 *
 * @param {number} epochNumber — the epoch to check
 * @param {number} genesisTimestamp
 * @param {number} epochDurationMs
 * @returns {boolean}
 */
function shouldFinalize(epochNumber, genesisTimestamp, epochDurationMs) {
  var nextEpochStart = epochStartTime(epochNumber + 1, genesisTimestamp, epochDurationMs);
  return Date.now() >= nextEpochStart;
}

/**
 * Deterministically choose the expected proposer for an epoch.
 * Uses the previous block hash as the seed so every node reaches the same
 * authority choice after replaying the same chain.
 */
function selectProposer(epochNumber, previousBlockHash, eligibleAccounts) {
  if (!Array.isArray(eligibleAccounts) || eligibleAccounts.length === 0) return null;
  var accounts = eligibleAccounts
    .filter(function (a) { return typeof a === "string" && a.length > 0; })
    .slice()
    .sort();
  if (accounts.length === 0) return null;
  var seed = String(previousBlockHash || "") + ":" + String(epochNumber || 0);
  var digest = crypto.createHash("sha256").update(seed).digest();
  var n = digest.readUInt32BE(0);
  return accounts[n % accounts.length];
}

/**
 * Validate a BLOCK_PROPOSAL proposer against deterministic authority rotation.
 * If the designated proposer misses most of the epoch, any eligible proposer
 * can finalize as a fallback so the chain self-heals instead of stalling.
 *
 * @returns {{valid:boolean, reason:string|null, designated:string|null, fallback:boolean}}
 */
function validateProposer(proposer, epochNumber, previousBlockHash, eligibleAccounts, epochStartMs, nowMs) {
  var designated = selectProposer(epochNumber, previousBlockHash, eligibleAccounts);
  if (!proposer) {
    return { valid: false, reason: "missing proposer", designated: designated, fallback: false };
  }
  if (!designated) {
    return { valid: true, reason: null, designated: proposer, fallback: true };
  }
  var accounts = eligibleAccounts || [];
  if (accounts.indexOf(proposer) < 0) {
    return { valid: false, reason: "proposer is not eligible", designated: designated, fallback: false };
  }
  if (proposer === designated) {
    return { valid: true, reason: null, designated: designated, fallback: false };
  }
  var epochMs = 30000;
  var envEpochMs = parseInt(process.env.BTCPC_EPOCH_DURATION_MS || process.env.EPOCH_DURATION_MS, 10);
  if (Number.isFinite(envEpochMs) && envEpochMs > 0) epochMs = envEpochMs;
  var start = Number(epochStartMs);
  var now = Number(nowMs || Date.now());
  var fallbackAfterMs = Math.floor(epochMs * 0.75);
  if (Number.isFinite(start) && start > 0 && now >= start + fallbackAfterMs) {
    return { valid: true, reason: null, designated: designated, fallback: true };
  }
  return { valid: false, reason: "not designated proposer (designated " + designated + ")", designated: designated, fallback: false };
}

/**
 * Compare two finalization proposals.
 * The one with more total work wins. Ties broken by earlier timestamp.
 *
 * @returns {number} -1 if a wins, 1 if b wins, 0 if equal
 */
function compareFinalization(a, b) {
  // More total work wins
  if ((a.total_work || 0) !== (b.total_work || 0)) {
    return (b.total_work || 0) - (a.total_work || 0);
  }
  // Earlier timestamp wins
  return (a.timestamp || 0) - (b.timestamp || 0);
}

/**
 * Validate an EPOCH_START message.
 * Checks: sender is registered and eligible, epoch number matches clock.
 */
function validateEpochStart(msg, nodeRegistry, genesisTimestamp, epochDurationMs) {
  var data = msg.data || {};
  if (!data.epoch_number && data.epoch_number !== 0) return false;
  if (!data.authority) return false;

  // Check sender is registered
  var nodeInfo = nodeRegistry.getNode(data.authority);
  var eligible = isEpochEligible(data.authority, nodeInfo, nodeRegistry.PERMISSIONLESS_MIN_STAKE);
  if (!eligible.eligible) return false;

  // Check epoch number is reasonable (within 1 of what our clock says)
  var clockEpoch = computeEpochNumber(genesisTimestamp, epochDurationMs);
  if (Math.abs(data.epoch_number - clockEpoch) > 1) return false;

  return true;
}

module.exports = {
  isEpochEligible: isEpochEligible,
  computeEpochNumber: computeEpochNumber,
  epochStartTime: epochStartTime,
  shouldFinalize: shouldFinalize,
  selectProposer: selectProposer,
  validateProposer: validateProposer,
  compareFinalization: compareFinalization,
  validateEpochStart: validateEpochStart,
  GENESIS_MINER: GENESIS_MINER
};

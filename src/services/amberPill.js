"use strict";

/**
 * Amber Pill — Geo-Pioneer Incentive System (v2.18)
 * Shin Devlin
 *
 * First node of each role to operate in a geographic region earns:
 *   1. A non-transferable (soulbound) Amber Pill NFT — permanent badge.
 *   2. A 1.5x reward weight multiplier for that role's pool in that region
 *      for the first year (~1,051,200 epochs at 30s/epoch).
 *
 * Region codes follow a hierarchical scheme: <country>-<area>-<locality>
 * The Amber Pill is awarded at the metro-area level (middle tier).
 *
 * Roles tracked independently:
 *   MINER, STORAGE, CLOCK, SENSOR, GATEWAY
 *
 * Each (role, region) pair has at most one pioneer. The second node in the
 * same role+region gets nothing. Different roles in the same region each
 * get their own pill.
 *
 * NFT IDs follow: amber-pill-<role>-<region> (lowercased, sanitised).
 * These IDs must be checked in the NFT_TRANSFER handler to block transfers.
 */

// In-memory pioneer registry.
// Key: "<role>:<region>" → { account, role, region, epoch, timestamp, expires_epoch, nft_id }
var pioneers = new Map();

// Epochs for a one-year bonus at 30-second epochs:
//   365.25 days × 24 h × 60 min × 2 epochs/min = 1,051,200 epochs
var BONUS_DURATION_EPOCHS = 1051200;

/**
 * Check whether a region already has a pioneer for a given role, and if
 * not, award the Amber Pill to `account`.
 *
 * Returns the new pill object if minted, or null if the slot is already
 * taken.
 *
 * @param {string} account   — HONE account name (e.g. "shindevlin")
 * @param {string} role      — one of MINER, STORAGE, CLOCK, SENSOR, GATEWAY
 * @param {string} region    — metro-area region code (e.g. "sv-san-salvador")
 * @param {number} epoch     — current chain epoch
 * @returns {{ account, role, region, epoch, timestamp, expires_epoch, nft_id }|null}
 */
function checkAndAwardPill(account, role, region, epoch) {
  if (!account || !role || !region) return null;

  var key = role + ":" + region;
  if (pioneers.has(key)) return null; // slot already taken

  var nft_id = "amber-pill-" + role.toLowerCase() + "-" + region.replace(/[^a-z0-9-]/g, "-");
  var pill = {
    account: account,
    role: role,
    region: region,
    epoch: epoch || 0,
    timestamp: Date.now(),
    expires_epoch: (epoch || 0) + BONUS_DURATION_EPOCHS,
    nft_id: nft_id,
  };
  pioneers.set(key, pill);
  return pill;
}

/**
 * Check whether `account` holds the Amber Pill for (role, region).
 *
 * @param {string} account
 * @param {string} role
 * @param {string} region
 * @returns {boolean}
 */
function hasAmberPill(account, role, region) {
  if (!account || !role || !region) return false;
  var key = role + ":" + region;
  var pill = pioneers.get(key);
  return !!(pill && pill.account === account);
}

/**
 * Return 1.5 if `account` holds a still-active Amber Pill for
 * (role, region), otherwise 1.0.
 *
 * @param {string} account
 * @param {string} role
 * @param {string} region
 * @param {number} currentEpoch
 * @returns {number} 1.5 or 1.0
 */
function getAmberPillWeight(account, role, region, currentEpoch) {
  if (!account || !role || !region) return 1.0;
  var key = role + ":" + region;
  var pill = pioneers.get(key);
  if (!pill || pill.account !== account) return 1.0;
  if ((currentEpoch || 0) - pill.epoch >= BONUS_DURATION_EPOCHS) return 1.0; // expired
  return 1.5;
}

/**
 * Get the Amber Pill record for a (role, region) pair, or null.
 *
 * @param {string} role
 * @param {string} region
 * @returns {object|null}
 */
function getAmberPill(role, region) {
  if (!role || !region) return null;
  return pioneers.get(role + ":" + region) || null;
}

/**
 * Return all awarded Amber Pills as an array.
 *
 * @returns {Array}
 */
function getAllAmberPills() {
  var result = [];
  for (var pill of pioneers.values()) {
    result.push(Object.assign({}, pill));
  }
  return result;
}

/**
 * Return all Amber Pills held by a specific account.
 *
 * @param {string} account
 * @returns {Array}
 */
function getAmberPillsByAccount(account) {
  if (!account) return [];
  var result = [];
  for (var pill of pioneers.values()) {
    if (pill.account === account) {
      result.push(Object.assign({}, pill));
    }
  }
  return result;
}

/**
 * Convenience wrapper: called when a heartbeat / proof arrives for a node.
 * Extracts region from the provided context and calls checkAndAwardPill.
 *
 * Returns the new pill if minted, or null.
 *
 * @param {string} account
 * @param {string} role  — MINER | STORAGE | CLOCK | SENSOR | GATEWAY
 * @param {string} region
 * @param {number} epoch
 * @returns {object|null}
 */
function checkHeartbeat(account, role, region, epoch) {
  if (!account || !role || !region) return null;
  return checkAndAwardPill(account, role, region, epoch);
}

/**
 * Load a pill from a persisted entry (called by stateStore replay).
 * Does NOT overwrite an existing pioneer for the same key — first writer wins.
 *
 * @param {object} pillData  — { account, role, region, epoch, timestamp, expires_epoch, nft_id }
 */
function loadFromEntry(pillData) {
  if (!pillData || !pillData.role || !pillData.region) return;
  var key = pillData.role + ":" + pillData.region;
  if (!pioneers.has(key)) {
    pioneers.set(key, {
      account: pillData.account,
      role: pillData.role,
      region: pillData.region,
      epoch: pillData.epoch || 0,
      timestamp: pillData.timestamp || 0,
      expires_epoch: pillData.expires_epoch || (pillData.epoch || 0) + BONUS_DURATION_EPOCHS,
      nft_id: pillData.nft_id || ("amber-pill-" + pillData.role.toLowerCase() + "-" + pillData.region.replace(/[^a-z0-9-]/g, "-")),
    });
  }
}

/**
 * Determine whether an NFT ID belongs to an Amber Pill (non-transferable).
 * Used by the NFT transfer handler to block soulbound amber pills.
 *
 * @param {string} nftId
 * @returns {boolean}
 */
function isAmberPillNftId(nftId) {
  return typeof nftId === "string" && nftId.startsWith("amber-pill-");
}

/**
 * Reset all state — for use in tests only.
 */
function resetForTests() {
  pioneers.clear();
}

module.exports = {
  checkAndAwardPill,
  checkHeartbeat,
  hasAmberPill,
  getAmberPillWeight,
  getAmberPill,
  getAllAmberPills,
  getAmberPillsByAccount,
  loadFromEntry,
  isAmberPillNftId,
  resetForTests,
  BONUS_DURATION_EPOCHS,
};

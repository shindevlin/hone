"use strict";

/**
 * BTCPC-FS Host Selector — v2.11.2
 * Shin Devlin
 *
 * Protocol-driven host selection for BLOB_STORE_COMMIT. Picks N active
 * + M cold hosts from the available pool based on reputation, uptime,
 * capacity, and region constraints. Best-effort: if the pool is smaller
 * than the target, returns everything available and flags the commit
 * as under-replicated.
 *
 * Pure function — no side effects, no chain writes. Caller is
 * responsible for passing the result into recordBlobStoreCommit.
 *
 * Scoring (higher = better):
 *
 *   score(host) = reputation * 1.0
 *               + uptime_factor * 50
 *               + capacity_score * 10
 *               - already_committed_blobs * 0.1
 *
 * Where:
 *   reputation         — host's storage_host reputation score (-100 to +100)
 *   uptime_factor      — 0.0 to 1.0 from STORAGE_HEARTBEAT history
 *   capacity_score     — log10(capacity_gb) to favor large-capacity hosts
 *                        without fully dominating
 *   already_committed  — light penalty to spread blobs across the fleet
 *                        and not pile everything onto the top few hosts
 *
 * Region filtering is applied BEFORE scoring — any host not matching
 * region_constraints is silently excluded from the candidate pool.
 */

/**
 * Select hosts for a new blob commit.
 *
 * @param {object} params
 * @param {number} params.target_active — desired active host count
 * @param {number} params.target_cold   — desired cold host count
 * @param {string[]} params.region_constraints — ISO region list (empty = any)
 * @param {number} params.current_epoch — used for uptime window
 * @param {object} params.stateStore — injected for testability
 * @returns {{
 *   active_hosts: string[],
 *   cold_hosts: string[],
 *   actual_active: number,
 *   actual_cold: number,
 *   under_replicated: boolean,
 *   candidates_considered: number,
 *   rejected_by_region: number,
 * }}
 */
function selectHosts(params) {
  params = params || {};
  var targetActive = Number.isFinite(params.target_active) ? params.target_active : 3;
  var targetCold = Number.isFinite(params.target_cold) ? params.target_cold : 5;
  var regionConstraints = Array.isArray(params.region_constraints)
    ? params.region_constraints.map(function (r) { return String(r).toUpperCase(); })
    : [];
  var currentEpoch = params.current_epoch || 0;
  var stateStore = params.stateStore || require("../chain/stateStore");

  // Candidate pool: all accounts that have declared any storage role
  // AND have heartbeated in the last 100 epochs.
  var activeHosts = stateStore.getActiveStorageHosts
    ? stateStore.getActiveStorageHosts(currentEpoch, 100)
    : [];

  var candidatesActive = [];
  var candidatesCold = [];
  var candidatesConsidered = 0;
  var rejectedByRegion = 0;

  for (var i = 0; i < activeHosts.length; i++) {
    var hostRecord = activeHosts[i];
    var hostName = hostRecord.host;
    candidatesConsidered++;

    // Read the account's capability declaration
    var account = stateStore.getAccount(hostName);
    if (!account || !account.node_types) continue;

    var canActive = account.node_types.indexOf("active_host") >= 0;
    var canCold = account.node_types.indexOf("cold_host") >= 0;
    if (!canActive && !canCold) continue;

    // Region filter
    if (regionConstraints.length > 0) {
      var hostRegion = account.lora_region || account.region || "";
      if (regionConstraints.indexOf(String(hostRegion).toUpperCase()) < 0) {
        rejectedByRegion++;
        continue;
      }
    }

    // Compute score
    var rep = stateStore.getReputation
      ? stateStore.getReputation("storage_host", hostName)
      : null;
    var repScore = rep ? rep.score || 0 : 0;

    var uptime = stateStore.getStorageUptimeFactor
      ? stateStore.getStorageUptimeFactor(hostName, currentEpoch, 100, 5)
      : 0;

    var capacityGb = account.storage_capacity_gb || 0;
    var capacityScore = capacityGb > 0 ? Math.log10(capacityGb + 1) : 0;

    // Existing commitment count (light anti-concentration pressure)
    var committed = stateStore.getBlobCommitsByHost
      ? stateStore.getBlobCommitsByHost(hostName).length
      : 0;

    var score =
      repScore * 1.0 +
      uptime * 50 +
      capacityScore * 10 -
      committed * 0.1;

    var candidate = {
      host: hostName,
      score: score,
      capacity_gb: capacityGb,
      uptime: uptime,
      reputation: repScore,
      committed_count: committed,
    };

    if (canActive) candidatesActive.push(candidate);
    if (canCold) candidatesCold.push(candidate);
  }

  // Sort descending by score
  candidatesActive.sort(function (a, b) { return b.score - a.score; });
  candidatesCold.sort(function (a, b) { return b.score - a.score; });

  // Best effort: take up to target, dedupe across the two lists so a
  // single host isn't picked for both active and cold slots on the
  // same commit.
  var selectedActive = [];
  var selectedCold = [];
  var picked = {};

  for (var j = 0; j < candidatesActive.length && selectedActive.length < targetActive; j++) {
    var c = candidatesActive[j];
    if (picked[c.host]) continue;
    selectedActive.push(c.host);
    picked[c.host] = true;
  }

  for (var k = 0; k < candidatesCold.length && selectedCold.length < targetCold; k++) {
    var cc = candidatesCold[k];
    if (picked[cc.host]) continue;
    selectedCold.push(cc.host);
    picked[cc.host] = true;
  }

  var underReplicated =
    selectedActive.length < targetActive || selectedCold.length < targetCold;

  return {
    active_hosts: selectedActive,
    cold_hosts: selectedCold,
    actual_active: selectedActive.length,
    actual_cold: selectedCold.length,
    target_active: targetActive,
    target_cold: targetCold,
    under_replicated: underReplicated,
    candidates_considered: candidatesConsidered,
    rejected_by_region: rejectedByRegion,
  };
}

/**
 * Scan existing blob commits for any that are under-replicated AND can
 * be brought closer to their targets by adding newly-available hosts.
 * Used by a background rebalancer at epoch boundaries.
 *
 * Returns an array of suggested commits to re-issue:
 *   [
 *     { cid, add_active: [...], add_cold: [...] },
 *     ...
 *   ]
 *
 * The caller records a fresh BLOB_STORE_COMMIT for each to extend the
 * host list.
 */
function findRebalanceOpportunities(params) {
  params = params || {};
  var currentEpoch = params.current_epoch || 0;
  var stateStore = params.stateStore || require("../chain/stateStore");
  var maxSuggestions = params.max_suggestions || 50;

  var all = stateStore.getAllBlobCommits ? stateStore.getAllBlobCommits() : [];
  var opportunities = [];

  for (var i = 0; i < all.length; i++) {
    if (opportunities.length >= maxSuggestions) break;
    var blob = all[i];
    if (!blob.under_replicated) continue;

    var activeNeeded = Math.max(0, (blob.target_active || 0) - (blob.active_hosts || []).length);
    var coldNeeded = Math.max(0, (blob.target_cold || 0) - (blob.cold_hosts || []).length);
    if (activeNeeded === 0 && coldNeeded === 0) continue;

    // Select additional hosts, excluding those already in the commit
    var selection = selectHosts({
      target_active: activeNeeded,
      target_cold: coldNeeded,
      region_constraints: blob.region_constraints || [],
      current_epoch: currentEpoch,
      stateStore: stateStore,
    });

    var existing = {};
    (blob.active_hosts || []).forEach(function (h) { existing[h] = true; });
    (blob.cold_hosts || []).forEach(function (h) { existing[h] = true; });

    var addActive = selection.active_hosts.filter(function (h) { return !existing[h]; });
    var addCold = selection.cold_hosts.filter(function (h) { return !existing[h]; });

    if (addActive.length > 0 || addCold.length > 0) {
      opportunities.push({
        cid: blob.cid,
        add_active: addActive,
        add_cold: addCold,
      });
    }
  }

  return opportunities;
}

module.exports = {
  selectHosts: selectHosts,
  findRebalanceOpportunities: findRebalanceOpportunities,
};

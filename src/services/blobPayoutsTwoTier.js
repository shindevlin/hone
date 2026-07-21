"use strict";

/**
 * HONE-FS Blob Payouts (Two-Tier) — v2.11.2-delta
 * Shin Devlin
 *
 * Implements the v2.11.2 two-tier active/cold payout split. Sibling
 * module to src/services/blobPayouts.js (v2.11.1 single-pool) — both
 * coexist so legacy commits keep working while new commits use the
 * uptime + bandwidth + challenge-success-weighted formula.
 *
 * Selection rule: if a blob has populated active_hosts or cold_hosts,
 * use the two-tier path. Otherwise fall back to the legacy single-pool
 * computePayouts in blobPayouts.js.
 *
 * Split (from docs/TOKENOMICS.md §5, v2.11.2 update):
 *   70% → active hosts
 *           weighted 70% by bytes_served + 30% by uptime_factor
 *           multiplied by challenge_success_rate
 *   20% → cold hosts
 *           weighted 100% by uptime_factor
 *           multiplied by challenge_success_rate
 *    9% → hone_recycle  (No Burn, All Recycle)
 *    1% → hone_reputation_pool
 *
 * CRITICAL — see feedback_storage_no_slash.md:
 * Failed challenges and low uptime REDUCE payout share but NEVER
 * slash the host's balance or stake. The unused portion of the pool
 * is left unpaid (caller can carry forward to next settlement).
 * Storage is pay-for-delivery, not slash-for-absence.
 */

var ACTIVE_HOST_SHARE = 0.70;
var COLD_HOST_SHARE = 0.20;
var RECYCLE_SHARE = 0.09;
var REPUTATION_SHARE = 0.01;

var ACTIVE_BANDWIDTH_WEIGHT = 0.70;
var ACTIVE_UPTIME_WEIGHT = 0.30;

function _round(n) {
  return parseFloat(Number(n).toFixed(10));
}

function _lookup(obj, key, fallback) {
  var v = obj && obj[key];
  if (typeof v === "number" && isFinite(v)) return v;
  return fallback;
}

/**
 * Pure function — compute two-tier payouts for a blob given context.
 *
 * @param {object} blobCommit — from stateStore.getBlobCommit(cid)
 * @param {object} [context] — pre-computed factors per host:
 *   {
 *     uptime_by_host: { [host]: 0.0-1.0 },
 *     success_by_host: { [host]: 0.0-1.0 },
 *   }
 * Missing factors default to 1.0 (full credit). This makes computePayouts
 * pure and testable independently of stateStore.
 *
 * @returns {object} payout breakdown with active_payouts, cold_payouts,
 *   pools, and a flattened host_payouts list.
 */
function computePayouts(blobCommit, context) {
  if (!blobCommit) return null;
  context = context || {};
  var uptimeByHost = context.uptime_by_host || {};
  var successByHost = context.success_by_host || {};

  var totalPool = Number(blobCommit.payment_hone) || 0;
  var activeHosts = Array.isArray(blobCommit.active_hosts) ? blobCommit.active_hosts : [];
  var coldHosts = Array.isArray(blobCommit.cold_hosts) ? blobCommit.cold_hosts : [];

  var activePool = _round(totalPool * ACTIVE_HOST_SHARE);
  var coldPool = _round(totalPool * COLD_HOST_SHARE);
  var recyclePool = _round(totalPool * RECYCLE_SHARE);
  var reputationPool = _round(totalPool * REPUTATION_SHARE);

  var servedByHost = blobCommit.bytes_served_by_host || {};

  // ── Active host weighting ──
  var totalBytes = 0;
  var totalActiveUptime = 0;
  for (var i = 0; i < activeHosts.length; i++) {
    totalBytes += Number(servedByHost[activeHosts[i]]) || 0;
    totalActiveUptime += _lookup(uptimeByHost, activeHosts[i], 1.0);
  }

  var activePayouts = activeHosts.map(function (host) {
    var bytes = Number(servedByHost[host]) || 0;
    var uptime = _lookup(uptimeByHost, host, 1.0);
    var success = _lookup(successByHost, host, 1.0);
    var bandwidthWeight = totalBytes > 0 ? bytes / totalBytes : 0;
    var uptimeWeight = totalActiveUptime > 0 ? uptime / totalActiveUptime : 0;
    var compositeWeight =
      ACTIVE_BANDWIDTH_WEIGHT * bandwidthWeight +
      ACTIVE_UPTIME_WEIGHT * uptimeWeight;
    var amount = _round(activePool * compositeWeight * success);
    return {
      host: host,
      role: "active",
      bytes_served: bytes,
      uptime_factor: uptime,
      success_rate: success,
      composite_weight: _round(compositeWeight),
      amount_hone: amount,
    };
  });

  // ── Cold host weighting ──
  var totalColdUptime = 0;
  for (var j = 0; j < coldHosts.length; j++) {
    totalColdUptime += _lookup(uptimeByHost, coldHosts[j], 1.0);
  }

  var coldPayouts = coldHosts.map(function (host) {
    var uptime = _lookup(uptimeByHost, host, 1.0);
    var success = _lookup(successByHost, host, 1.0);
    var weight = totalColdUptime > 0 ? uptime / totalColdUptime : 0;
    var amount = _round(coldPool * weight * success);
    return {
      host: host,
      role: "cold",
      uptime_factor: uptime,
      success_rate: success,
      weight: _round(weight),
      amount_hone: amount,
    };
  });

  var totalHostPayout = 0;
  activePayouts.forEach(function (p) { totalHostPayout += p.amount_hone; });
  coldPayouts.forEach(function (p) { totalHostPayout += p.amount_hone; });

  return {
    cid: blobCommit.cid,
    version: "2.11.2",
    total_pool: totalPool,
    active_pool: activePool,
    cold_pool: coldPool,
    recycle_pool: recyclePool,
    reputation_pool: reputationPool,
    total_bytes_served: totalBytes,
    total_active_uptime: _round(totalActiveUptime),
    total_cold_uptime: _round(totalColdUptime),
    active_payouts: activePayouts,
    cold_payouts: coldPayouts,
    total_host_payout: _round(totalHostPayout),
    host_payouts: activePayouts.concat(coldPayouts),
  };
}

/**
 * End-to-end settlement: pull factors from stateStore, compute payouts,
 * record HONE transfers via ledger.recordTransfer.
 *
 * Dependencies injected for testability.
 */
async function settlePayouts(blobCommit, options) {
  options = options || {};
  var ledger = options.ledger || require("./ledger");
  var stateStore = options.stateStore || require("../chain/stateStore");
  var uploader = options.uploader || (blobCommit && blobCommit.uploader);
  var epoch = options.epoch;
  var currentEpoch = options.current_epoch;

  if (!Number.isFinite(epoch)) {
    epoch = await ledger.getCurrentEpoch();
  }
  if (!Number.isFinite(currentEpoch)) currentEpoch = epoch;

  if (!blobCommit) return { transfers: [] };

  // Build context: per-host uptime + challenge success from stateStore
  var context = { uptime_by_host: {}, success_by_host: {} };
  var allHosts = [];
  if (Array.isArray(blobCommit.active_hosts)) allHosts = allHosts.concat(blobCommit.active_hosts);
  if (Array.isArray(blobCommit.cold_hosts)) allHosts = allHosts.concat(blobCommit.cold_hosts);
  for (var h = 0; h < allHosts.length; h++) {
    var host = allHosts[h];
    context.uptime_by_host[host] = stateStore.getStorageUptimeFactor
      ? stateStore.getStorageUptimeFactor(host, currentEpoch, 100, 5)
      : 1.0;
    context.success_by_host[host] = stateStore.getChallengeSuccessRate
      ? stateStore.getChallengeSuccessRate(host)
      : 1.0;
  }

  var calc = computePayouts(blobCommit, context);
  if (!calc) return { transfers: [] };
  if (calc.total_pool <= 0) return Object.assign({ transfers: [] }, calc);
  if (!uploader) throw new Error("uploader required for settlement");

  var transfers = [];

  // Pay each active host
  for (var i = 0; i < calc.active_payouts.length; i++) {
    var ap = calc.active_payouts[i];
    if (ap.amount_hone > 0) {
      await ledger.recordTransfer(
        uploader,
        ap.host,
        ap.amount_hone,
        "HONE",
        null,
        epoch,
        "Blob payout (active): " + calc.cid
      );
      transfers.push({ to: ap.host, amount: ap.amount_hone, kind: "host_active" });
    }
  }

  // Pay each cold host
  for (var k = 0; k < calc.cold_payouts.length; k++) {
    var cp = calc.cold_payouts[k];
    if (cp.amount_hone > 0) {
      await ledger.recordTransfer(
        uploader,
        cp.host,
        cp.amount_hone,
        "HONE",
        null,
        epoch,
        "Blob payout (cold): " + calc.cid
      );
      transfers.push({ to: cp.host, amount: cp.amount_hone, kind: "host_cold" });
    }
  }

  // Recycle share — always paid (No Burn, All Recycle)
  if (calc.recycle_pool > 0) {
    await ledger.recordTransfer(
      uploader,
      "hone_recycle",
      calc.recycle_pool,
      "HONE",
      null,
      epoch,
      "Blob payout (recycle): " + calc.cid
    );
    transfers.push({ to: "hone_recycle", amount: calc.recycle_pool, kind: "recycle" });
  }

  // Reputation pool share — always paid
  if (calc.reputation_pool > 0) {
    await ledger.recordTransfer(
      uploader,
      "hone_reputation_pool",
      calc.reputation_pool,
      "HONE",
      null,
      epoch,
      "Blob payout (reputation): " + calc.cid
    );
    transfers.push({
      to: "hone_reputation_pool",
      amount: calc.reputation_pool,
      kind: "reputation",
    });
  }

  return Object.assign({ transfers: transfers }, calc);
}

module.exports = {
  ACTIVE_HOST_SHARE: ACTIVE_HOST_SHARE,
  COLD_HOST_SHARE: COLD_HOST_SHARE,
  RECYCLE_SHARE: RECYCLE_SHARE,
  REPUTATION_SHARE: REPUTATION_SHARE,
  ACTIVE_BANDWIDTH_WEIGHT: ACTIVE_BANDWIDTH_WEIGHT,
  ACTIVE_UPTIME_WEIGHT: ACTIVE_UPTIME_WEIGHT,
  computePayouts: computePayouts,
  settlePayouts: settlePayouts,
};

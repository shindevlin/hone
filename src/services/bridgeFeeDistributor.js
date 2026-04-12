"use strict";

/**
 * BTCPC Bridge Fee Distributor — v2.16-alpha
 * Shin Devlin
 *
 * Pure fee calculation and LP distribution module for the lock-and-recycle
 * bridge. Mirrors the blobPayoutsTwoTier pattern — pure functions, no I/O,
 * dependencies injected for testability.
 *
 * Fee schedule (source asset, confirmed — see memory/feedback_no_burn_all_recycle.md):
 *   Wrap (all sizes):       0.05% flat — encourages adoption
 *   Unwrap < 1,000 BTCPC:   0.20%
 *   Unwrap 1,000-100,000:   0.15%
 *   Unwrap > 100,000:       0.10%
 *
 * Fee currency:
 *   Wrap fee in BTCPC (source asset)
 *   Unwrap fee in wBTCPC (source asset)
 *
 * Fee distribution to LPs:
 *   Pro-rata by weight = amount × remaining_lock_days (veCRV-style).
 *   10-20% of fees → smoothing buffer (default: 15% midpoint).
 *
 * No burn, no mint — all fees route to LPs or smoothing buffer.
 */

/** Wrap fee: flat 0.05% (5 basis points). */
var WRAP_FEE_BPS = 5;

/** Unwrap fee tiers. */
var UNWRAP_SMALL_THRESHOLD = 1000;
var UNWRAP_LARGE_THRESHOLD = 100000;
var UNWRAP_SMALL_BPS = 20;   // 0.20%
var UNWRAP_MID_BPS = 15;     // 0.15%
var UNWRAP_LARGE_BPS = 10;   // 0.10%

/** Fraction of fees routed to smoothing buffer (10-20%, midpoint 15%). */
var SMOOTHING_FRACTION = 0.15;

function _round(n) {
  return parseFloat(Number(n).toFixed(10));
}

/**
 * Compute the wrap fee for a given BTCPC amount.
 * Flat 0.05% regardless of size.
 *
 * @param {number} amount — gross BTCPC being wrapped (including fee)
 * @returns {{ fee, net, rate_bps }}
 */
function computeWrapFee(amount) {
  var a = Number(amount);
  if (!Number.isFinite(a) || a <= 0) {
    throw new Error("amount must be a positive finite number");
  }
  var fee = _round((WRAP_FEE_BPS / 10000) * a);
  var net = _round(a - fee);
  return { fee: fee, net: net, rate_bps: WRAP_FEE_BPS };
}

/**
 * Compute the unwrap fee for a given wBTCPC amount.
 * Tiered by volume:
 *   < 1,000   → 0.20%
 *   1,000-100,000 → 0.15%
 *   > 100,000 → 0.10%
 *
 * @param {number} amount — gross wBTCPC being unwrapped (including fee)
 * @returns {{ fee, net, tier, rate_bps }}
 */
function computeUnwrapFee(amount) {
  var a = Number(amount);
  if (!Number.isFinite(a) || a <= 0) {
    throw new Error("amount must be a positive finite number");
  }

  var bps;
  var tier;

  if (a < UNWRAP_SMALL_THRESHOLD) {
    bps = UNWRAP_SMALL_BPS;
    tier = "small";
  } else if (a <= UNWRAP_LARGE_THRESHOLD) {
    bps = UNWRAP_MID_BPS;
    tier = "mid";
  } else {
    bps = UNWRAP_LARGE_BPS;
    tier = "large";
  }

  var fee = _round((bps / 10000) * a);
  var net = _round(a - fee);
  return { fee: fee, net: net, tier: tier, rate_bps: bps };
}

/**
 * Distribute fees to LPs proportional to their veCRV-style weight.
 *
 * Pro-rata weight = amount × remaining_lock_days.
 * LPs with zero or expired weight receive no share.
 *
 * @param {string} chainId
 * @param {number} feeAmount — total fee amount to distribute
 * @param {string} feeCurrency — "BTCPC" or "wBTCPC"
 * @param {object} bridgeRegistry — injected bridgeRegistry module
 * @returns {Array<{ funder, weight, share, amount, currency }>}
 */
function distributeFeesToLPs(chainId, feeAmount, feeCurrency, bridgeRegistry) {
  if (!chainId) throw new Error("chainId required");
  if (!Number.isFinite(Number(feeAmount)) || Number(feeAmount) <= 0) {
    return [];
  }
  if (!bridgeRegistry || typeof bridgeRegistry.getAllFunders !== "function") {
    throw new Error("bridgeRegistry required with getAllFunders()");
  }

  var totalFee = Number(feeAmount);
  var funders = bridgeRegistry.getAllFunders(chainId);

  // Compute weights for all active (non-expired, non-queued) funders
  var activeFunders = funders.filter(function (f) {
    return f.current_weight > 0;
  });

  if (activeFunders.length === 0) {
    return [];
  }

  var totalWeight = 0;
  activeFunders.forEach(function (f) {
    totalWeight += f.current_weight;
  });

  var distributions = activeFunders.map(function (f) {
    var share = totalWeight > 0 ? f.current_weight / totalWeight : 0;
    var amount = _round(totalFee * share);
    return {
      funder: f.funder,
      weight: _round(f.current_weight),
      share: _round(share),
      amount: amount,
      currency: feeCurrency,
    };
  });

  return distributions;
}

/**
 * Accrue a portion of fees into the smoothing buffer.
 * 15% of fees (midpoint of 10-20% range) go to the smoothing buffer.
 * Delegates to bridgeRegistry.accrueSmoothingBuffer.
 *
 * @param {string} chainId
 * @param {number} feeAmount — total fee to allocate from
 * @param {object} bridgeRegistry — injected bridgeRegistry module
 * @returns {{ buffered, lp_distributable, smoothing_fraction }}
 */
function accrueSmoothingBuffer(chainId, feeAmount, bridgeRegistry) {
  if (!chainId) throw new Error("chainId required");
  if (!bridgeRegistry || typeof bridgeRegistry.accrueSmoothingBuffer !== "function") {
    throw new Error("bridgeRegistry required with accrueSmoothingBuffer()");
  }

  var fee = Number(feeAmount);
  if (!Number.isFinite(fee) || fee <= 0) {
    return { buffered: 0, lp_distributable: 0, smoothing_fraction: SMOOTHING_FRACTION };
  }

  var buffered = _round(fee * SMOOTHING_FRACTION);
  var lpDistributable = _round(fee - buffered);

  bridgeRegistry.accrueSmoothingBuffer(chainId, buffered);

  return {
    buffered: buffered,
    lp_distributable: lpDistributable,
    smoothing_fraction: SMOOTHING_FRACTION,
  };
}

module.exports = {
  // Constants
  WRAP_FEE_BPS: WRAP_FEE_BPS,
  UNWRAP_SMALL_THRESHOLD: UNWRAP_SMALL_THRESHOLD,
  UNWRAP_LARGE_THRESHOLD: UNWRAP_LARGE_THRESHOLD,
  UNWRAP_SMALL_BPS: UNWRAP_SMALL_BPS,
  UNWRAP_MID_BPS: UNWRAP_MID_BPS,
  UNWRAP_LARGE_BPS: UNWRAP_LARGE_BPS,
  SMOOTHING_FRACTION: SMOOTHING_FRACTION,

  // Core fee math
  computeWrapFee: computeWrapFee,
  computeUnwrapFee: computeUnwrapFee,

  // Distribution
  distributeFeesToLPs: distributeFeesToLPs,
  accrueSmoothingBuffer: accrueSmoothingBuffer,
};

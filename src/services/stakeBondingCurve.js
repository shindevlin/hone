"use strict";

/**
 * BTCPC Stake Bonding Curve — store capacity pricing.
 * Shin Devlin
 *
 * Linear bonding curve: the Nth slot costs more than the first.
 * Cheap at the low end (hobbyist sellers), expensive at scale (prevents
 * monopoly flooding of the chain with spam listings).
 *
 *   price_per_slot(n) = BASE + SLOPE * n        (units: USD)
 *   total_cost(0, n)  = BASE * n + SLOPE * (n-1) * n / 2
 *
 * The integral form lets us answer:
 *   "How much USD to add X slots on top of an existing capacity of Y?"
 *   "How many slots does a USD payment of Z buy at current capacity?"
 *
 * BASE and SLOPE are chain parameters. They can be tuned via future
 * governance without changing this file's API.
 *
 * STAKE vs CAPACITY vs FEES (three separate things):
 *   - CAPACITY: how many products a store can list.
 *   - STAKE: BTCPC locked as collateral (slashable for fraud).
 *            capacity * STAKE_PER_SLOT_BTCPC locked.
 *   - FEES:  per-epoch BTCPC-FS hosting fees (pay-as-you-go, separate).
 *
 * The bonding curve prices CAPACITY purchases in USD. The payment is made
 * in a wrapped stable (wUSDC/wUSDT/wDAI) on-chain, and the equivalent
 * BTCPC stake is locked from the seller's own BTCPC balance. The stable
 * payment goes to the protocol treasury; the stake sits in escrow until
 * store close (minus any slashing).
 */

// Default curve parameters. Tunable — can be overridden at call sites.
var DEFAULT_BASE_USD = 1.00;       // first slot costs $1
var DEFAULT_SLOPE_USD = 0.05;      // each additional slot adds $0.05 to price
var DEFAULT_STAKE_PER_SLOT_BTCPC = 1; // 1 BTCPC locked per slot of capacity

/**
 * Price in USD to add `units` slots on top of `currentCapacity`.
 * Integral of (base + slope*n) from currentCapacity to currentCapacity+units.
 */
function costForCapacity(currentCapacity, units, opts) {
  opts = opts || {};
  var base = typeof opts.base === "number" ? opts.base : DEFAULT_BASE_USD;
  var slope = typeof opts.slope === "number" ? opts.slope : DEFAULT_SLOPE_USD;

  if (units <= 0) return 0;
  var start = currentCapacity;
  var end = currentCapacity + units;

  // Sum of arithmetic series: base*units + slope * (sum of n for n=start..end-1)
  // sum of n from a..b-1 = (b-1)*b/2 - (a-1)*a/2 = (b^2 - b - a^2 + a) / 2
  var sliceSum = ((end - 1) * end - (start - 1) * start) / 2;
  var total = base * units + slope * sliceSum;
  return _round(total);
}

/**
 * How many slots a USD payment buys at the current capacity. Inverse of
 * costForCapacity — solves the quadratic for the largest N where
 * costForCapacity(currentCapacity, N) <= paymentUSD.
 */
function capacityForPayment(currentCapacity, paymentUSD, opts) {
  opts = opts || {};
  if (paymentUSD <= 0) return 0;

  var base = typeof opts.base === "number" ? opts.base : DEFAULT_BASE_USD;
  var slope = typeof opts.slope === "number" ? opts.slope : DEFAULT_SLOPE_USD;

  // Solve: base*n + slope * ((start+n-1)*(start+n) - (start-1)*start) / 2 <= paymentUSD
  // Easier: linear search with early exit. Store capacity is small (< 1M realistic).
  // Exact binary search for N such that costForCapacity(start, N) <= paymentUSD < costForCapacity(start, N+1)
  var lo = 0;
  var hi = 1;
  // Expand hi until cost exceeds payment
  while (costForCapacity(currentCapacity, hi, { base: base, slope: slope }) <= paymentUSD) {
    lo = hi;
    hi = hi * 2;
    if (hi > 10000000) break; // sanity cap
  }
  // Binary search
  while (lo < hi - 1) {
    var mid = Math.floor((lo + hi) / 2);
    var cost = costForCapacity(currentCapacity, mid, { base: base, slope: slope });
    if (cost <= paymentUSD) lo = mid;
    else hi = mid;
  }
  return lo;
}

/**
 * BTCPC collateral required to hold `totalCapacity` slots active.
 * Separate from the USD bonding curve price — this is the slashable bond.
 */
function stakeForCapacity(totalCapacity, opts) {
  opts = opts || {};
  var perSlot = typeof opts.stakePerSlot === "number" ? opts.stakePerSlot : DEFAULT_STAKE_PER_SLOT_BTCPC;
  return _round(totalCapacity * perSlot);
}

function _round(n) {
  return parseFloat(Number(n).toFixed(10));
}

module.exports = {
  DEFAULT_BASE_USD: DEFAULT_BASE_USD,
  DEFAULT_SLOPE_USD: DEFAULT_SLOPE_USD,
  DEFAULT_STAKE_PER_SLOT_BTCPC: DEFAULT_STAKE_PER_SLOT_BTCPC,
  costForCapacity: costForCapacity,
  capacityForPayment: capacityForPayment,
  stakeForCapacity: stakeForCapacity,
};

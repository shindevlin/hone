"use strict";

/**
 * HONE Bridge Registry — v2.16-alpha
 * Shin Devlin
 *
 * In-memory registry for the lock-and-recycle cross-chain bridge primitive.
 * Mirrors the serviceRegistry/oracleFeeds pattern — standalone primitive,
 * chain-integrated ledger entries land in a follow-up.
 *
 * Bridge design (all locked in — see memory/feedback_no_burn_all_recycle.md):
 *
 *   Per-chain wHONE supply: 4,200,000 hard cap (pre-minted at deploy, never
 *   minted again). No burn function. All unwraps are "transfer to bridge
 *   reserve", not burns. This is the HONE Lock-and-Recycle bridge.
 *
 *   Wrap flow:  user locks HONE native → wHONE released from reserve
 *   Unwrap flow: user transfers wHONE to reserve → HONE native released
 *
 *   LP funding: permissionless, variable lock period (30-1460 days).
 *   LP weight: amount × remaining_lock_days (veCRV-style).
 *   Withdrawal: FIFO queue drained by unwrap volume + 10-20% smoothing buffer.
 *
 *   Fee tiers (source asset, confirmed by user):
 *     Wrap (all sizes): 0.05%
 *     Unwrap < 1,000:   0.20%
 *     Unwrap 1,000-100,000: 0.15%
 *     Unwrap > 100,000: 0.10%
 */

// chainId → chain record
var chains = new Map();

// "chainId|funder" → funder lock record
var funders = new Map();

// chainId → FIFO withdrawal queue (array of funder names)
var withdrawalQueues = new Map();

// chainId → smoothing buffer amount (HONE)
var smoothingBuffers = new Map();

// chainId → { wraps, unwraps } (running totals)
var volumeStats = new Map();

/** wHONE hard cap per destination chain (4.2M, NOT 42M). */
var WHONE_SUPPLY_PER_CHAIN = 4200000;

/** Lock period bounds in days. */
var MIN_LOCK_DAYS = 30;
var MAX_LOCK_DAYS = 1460;

/** Smoothing buffer fraction of fees (10-20%, using 15% midpoint). */
var SMOOTHING_FEE_FRACTION = 0.15;

/** Smoothing buffer hard cap: ~10% of total locked liquidity. */
var SMOOTHING_CAP_FRACTION = 0.10;

function _round(n) {
  return parseFloat(Number(n).toFixed(10));
}

function _now() {
  return Date.now();
}

function _funderKey(chainId, funder) {
  return chainId + "|" + funder;
}

/**
 * Register a destination chain for bridging.
 *
 * @param {string} chainId — unique chain identifier (e.g. "base", "arbitrum", "ethereum")
 * @param {object} config
 * @param {string} config.name — human-readable chain name
 * @param {number} [config.whone_supply] — wHONE supply (defaults to WHONE_SUPPLY_PER_CHAIN)
 * @param {string} config.bridge_reserve_address — on-chain address of the bridge reserve
 * @param {number} [config.fee_wrap_bps] — wrap fee in basis points (default 5 = 0.05%)
 * @param {object} [config.fee_unwrap_tiers] — unwrap fee tiers (defaults to spec)
 * @param {number} [config.lock_period_min_days] — minimum LP lock days (default 30)
 * @param {number} [config.lock_period_max_days] — maximum LP lock days (default 1460)
 */
function registerChain(chainId, config) {
  if (typeof chainId !== "string" || chainId.length === 0) {
    throw new Error("chainId required");
  }
  if (!config || typeof config !== "object") {
    throw new Error("config required");
  }
  if (typeof config.name !== "string" || config.name.length === 0) {
    throw new Error("config.name required");
  }
  if (typeof config.bridge_reserve_address !== "string" || config.bridge_reserve_address.length === 0) {
    throw new Error("config.bridge_reserve_address required");
  }

  var supply = typeof config.whone_supply === "number"
    ? config.whone_supply
    : WHONE_SUPPLY_PER_CHAIN;

  if (supply !== WHONE_SUPPLY_PER_CHAIN) {
    // Allow override in tests but enforce the hard cap invariant
    if (supply > WHONE_SUPPLY_PER_CHAIN) {
      throw new Error(
        "wHONE supply per chain cannot exceed hard cap of " + WHONE_SUPPLY_PER_CHAIN +
        " (4.2M). It is 4.2M, not 42M."
      );
    }
  }

  var feeWrapBps = typeof config.fee_wrap_bps === "number" ? config.fee_wrap_bps : 5;
  var feeUnwrapTiers = config.fee_unwrap_tiers || {
    small_threshold: 1000,
    large_threshold: 100000,
    small_bps: 20,   // 0.20% for < 1,000
    mid_bps: 15,     // 0.15% for 1,000-100,000
    large_bps: 10,   // 0.10% for > 100,000
  };
  var lockMin = typeof config.lock_period_min_days === "number"
    ? config.lock_period_min_days
    : MIN_LOCK_DAYS;
  var lockMax = typeof config.lock_period_max_days === "number"
    ? config.lock_period_max_days
    : MAX_LOCK_DAYS;

  var existing = chains.get(chainId);
  var record = {
    chain_id: chainId,
    name: config.name,
    whone_supply: supply,
    bridge_reserve_address: config.bridge_reserve_address,
    fee_wrap_bps: feeWrapBps,
    fee_unwrap_tiers: feeUnwrapTiers,
    lock_period_min_days: lockMin,
    lock_period_max_days: lockMax,
    registered_at: existing ? existing.registered_at : _now(),
    updated_at: _now(),
    // Running state
    total_locked_hone: existing ? existing.total_locked_hone : 0,
    circulating_whone: existing ? existing.circulating_whone : 0,
  };

  chains.set(chainId, record);

  if (!smoothingBuffers.has(chainId)) smoothingBuffers.set(chainId, 0);
  if (!withdrawalQueues.has(chainId)) withdrawalQueues.set(chainId, []);
  if (!volumeStats.has(chainId)) volumeStats.set(chainId, { wraps: 0, unwraps: 0 });

  return Object.assign({}, record);
}

/**
 * Return a chain config + current state snapshot. Returns null if unknown.
 */
function getChain(chainId) {
  var rec = chains.get(chainId);
  if (!rec) return null;
  return Object.assign({}, rec);
}

/**
 * Return all registered chains.
 */
function getAllChains() {
  var result = [];
  chains.forEach(function (rec) {
    result.push(Object.assign({}, rec));
  });
  return result;
}

/**
 * Fund the bridge for a destination chain. Funder locks HONE native.
 * Returns the funder lock record including veCRV-style weight.
 *
 * @param {string} funder — HONE account name
 * @param {string} chainId
 * @param {number} amountHone — HONE to lock
 * @param {number} lockDays — lock period (30-1460)
 * @returns {{ funder, chain_id, amount, lock_days, lock_expires_at, weight, locked_at }}
 */
function fundBridge(funder, chainId, amountHone, lockDays) {
  if (typeof funder !== "string" || funder.length === 0) throw new Error("funder required");
  var chain = chains.get(chainId);
  if (!chain) throw new Error("unknown chainId: " + chainId);

  var amount = Number(amountHone);
  if (!Number.isFinite(amount) || amount <= 0) throw new Error("amountHone must be a positive number");

  var days = Number(lockDays);
  if (!Number.isFinite(days) || days < chain.lock_period_min_days || days > chain.lock_period_max_days) {
    throw new Error(
      "lockDays must be " + chain.lock_period_min_days + "-" + chain.lock_period_max_days
    );
  }

  // Check wHONE supply cap: circulating + new amount cannot exceed chain supply
  var newCirculating = chain.circulating_whone + amount;
  if (newCirculating > chain.whone_supply) {
    throw new Error(
      "bridge funding would exceed wHONE supply cap of " + chain.whone_supply +
      " (4.2M hard cap per chain)"
    );
  }

  var key = _funderKey(chainId, funder);
  var existing = funders.get(key);

  var now = _now();
  var lockExpiresAt = now + days * 86400 * 1000;

  var record;
  if (existing) {
    // Additional funding — add to existing position, reset lock to whichever is longer
    var newAmount = _round(existing.amount + amount);
    var newExpiry = Math.max(existing.lock_expires_at, lockExpiresAt);
    var newDays = Math.round((newExpiry - now) / (86400 * 1000));
    record = {
      funder: funder,
      chain_id: chainId,
      amount: newAmount,
      lock_days: newDays,
      locked_at: existing.locked_at,
      lock_expires_at: newExpiry,
      weight: _round(newAmount * newDays),
    };
  } else {
    record = {
      funder: funder,
      chain_id: chainId,
      amount: _round(amount),
      lock_days: Math.round(days),
      locked_at: now,
      lock_expires_at: lockExpiresAt,
      weight: _round(amount * Math.round(days)),
    };
  }

  funders.set(key, record);

  // Update chain state
  chain.total_locked_hone = _round(chain.total_locked_hone + amount);
  chain.circulating_whone = _round(chain.circulating_whone + amount);

  return Object.assign({}, record);
}

/**
 * Request unlock for a funder. Adds them to the FIFO withdrawal queue
 * if lock period has expired. Funders in queue continue earning fees.
 *
 * @returns {{ queued: bool, position: number, reason?: string }}
 */
function requestUnlock(funder, chainId) {
  if (typeof funder !== "string" || funder.length === 0) throw new Error("funder required");
  var chain = chains.get(chainId);
  if (!chain) throw new Error("unknown chainId: " + chainId);

  var key = _funderKey(chainId, funder);
  var lock = funders.get(key);
  if (!lock) throw new Error("no lock found for funder " + funder + " on chain " + chainId);

  var now = _now();
  if (now < lock.lock_expires_at) {
    return {
      queued: false,
      position: -1,
      reason: "lock has not expired yet, expires at " + new Date(lock.lock_expires_at).toISOString(),
    };
  }

  var queue = withdrawalQueues.get(chainId);
  if (queue.indexOf(funder) >= 0) {
    return {
      queued: true,
      position: queue.indexOf(funder) + 1,
      reason: "already in queue",
    };
  }

  queue.push(funder);
  return { queued: true, position: queue.length };
}

/**
 * Process the withdrawal queue. Drains FIFO positions using incoming
 * unwrap volume + smoothing buffer. Returns array of fulfilled withdrawals.
 *
 * @param {string} chainId
 * @param {number} [availableVolume=0] — HONE available from recent unwrap volume
 * @returns {Array<{ funder, amount, source }>}
 */
function processUnlock(chainId, availableVolume) {
  var chain = chains.get(chainId);
  if (!chain) throw new Error("unknown chainId: " + chainId);

  var queue = withdrawalQueues.get(chainId);
  var buffer = smoothingBuffers.get(chainId) || 0;
  var remaining = Number(availableVolume) || 0;
  var fulfilled = [];

  while (queue.length > 0 && (remaining > 0 || buffer > 0)) {
    var funder = queue[0];
    var key = _funderKey(chainId, funder);
    var lock = funders.get(key);
    if (!lock) {
      // Stale queue entry — remove and continue
      queue.shift();
      continue;
    }

    var needed = lock.amount;
    var source = "queue";
    var paid = 0;

    if (remaining >= needed) {
      paid = needed;
      remaining = _round(remaining - needed);
      source = "unwrap_volume";
    } else if (remaining + buffer >= needed) {
      // Draw from smoothing buffer to top up
      var fromBuffer = _round(needed - remaining);
      paid = needed;
      buffer = _round(buffer - fromBuffer);
      remaining = 0;
      source = "hybrid";
    } else {
      // Cannot fully satisfy — partial fills not supported (wait for more volume)
      break;
    }

    // Fulfill withdrawal
    queue.shift();
    funders.delete(key);
    chain.total_locked_hone = _round(Math.max(0, chain.total_locked_hone - lock.amount));
    chain.circulating_whone = _round(Math.max(0, chain.circulating_whone - lock.amount));

    fulfilled.push({ funder: funder, amount: paid, source: source });
  }

  smoothingBuffers.set(chainId, _round(buffer));
  return fulfilled;
}

/**
 * Record a wrap: user locks HONE, wHONE is released from bridge reserve.
 * Fee charged in HONE (source asset).
 *
 * @param {string} user
 * @param {string} chainId
 * @param {number} amountHone — gross amount including fee
 * @returns {{ user, chain_id, gross_amount, fee, net_amount, fee_currency, tx_hash_placeholder }}
 */
function wrap(user, chainId, amountHone) {
  if (typeof user !== "string" || user.length === 0) throw new Error("user required");
  var chain = chains.get(chainId);
  if (!chain) throw new Error("unknown chainId: " + chainId);

  var gross = Number(amountHone);
  if (!Number.isFinite(gross) || gross <= 0) throw new Error("amountHone must be positive");

  var feeInfo = getWrapFee(chainId, gross);

  // Circulating check: net_amount of wHONE must not exceed supply cap
  var afterCirculating = chain.circulating_whone + feeInfo.net;
  if (afterCirculating > chain.whone_supply) {
    throw new Error(
      "wrap would exceed wHONE circulating cap of " + chain.whone_supply +
      " for chain " + chainId
    );
  }

  chain.circulating_whone = _round(afterCirculating);

  var stats = volumeStats.get(chainId);
  stats.wraps = _round(stats.wraps + gross);

  var txHash = "wrap_" + chainId + "_" + user + "_" + _now();

  return {
    user: user,
    chain_id: chainId,
    gross_amount: gross,
    fee: feeInfo.fee,
    net_amount: feeInfo.net,
    fee_currency: "HONE",
    tx_hash_placeholder: txHash,
  };
}

/**
 * Record an unwrap: user transfers wHONE to bridge reserve, HONE released.
 * Fee charged in wHONE (source asset). No burn — transfer to reserve only.
 *
 * @param {string} user
 * @param {string} chainId
 * @param {number} amountWhone — gross amount including fee
 * @returns {{ user, chain_id, gross_amount, fee, net_amount, fee_currency, tier, tx_hash_placeholder }}
 */
function unwrap(user, chainId, amountWhone) {
  if (typeof user !== "string" || user.length === 0) throw new Error("user required");
  var chain = chains.get(chainId);
  if (!chain) throw new Error("unknown chainId: " + chainId);

  var gross = Number(amountWhone);
  if (!Number.isFinite(gross) || gross <= 0) throw new Error("amountWhone must be positive");

  if (gross > chain.circulating_whone) {
    throw new Error(
      "unwrap amount " + gross + " exceeds circulating wHONE " + chain.circulating_whone +
      " — cannot unwrap more than is circulating"
    );
  }

  var feeInfo = getUnwrapFee(chainId, gross);

  // wHONE transfers to reserve — reduce circulating by gross (full amount returns to reserve)
  chain.circulating_whone = _round(Math.max(0, chain.circulating_whone - gross));

  var stats = volumeStats.get(chainId);
  stats.unwraps = _round(stats.unwraps + gross);

  var txHash = "unwrap_" + chainId + "_" + user + "_" + _now();

  return {
    user: user,
    chain_id: chainId,
    gross_amount: gross,
    fee: feeInfo.fee,
    net_amount: feeInfo.net,
    fee_currency: "wHONE",
    tier: feeInfo.tier,
    tx_hash_placeholder: txHash,
  };
}

/**
 * Compute the wrap fee for a given amount.
 * Flat 0.05% for all sizes (fee_wrap_bps = 5).
 *
 * @param {string} chainId
 * @param {number} amount
 * @returns {{ fee, net, rate_bps }}
 */
function getWrapFee(chainId, amount) {
  var chain = chains.get(chainId);
  if (!chain) throw new Error("unknown chainId: " + chainId);
  var bps = chain.fee_wrap_bps;
  var fee = _round((bps / 10000) * amount);
  var net = _round(amount - fee);
  return { fee: fee, net: net, rate_bps: bps };
}

/**
 * Compute the unwrap fee for a given amount.
 * Tiered by volume: <1,000 = 0.20%, 1,000-100,000 = 0.15%, >100,000 = 0.10%.
 *
 * @param {string} chainId
 * @param {number} amount
 * @returns {{ fee, net, tier, rate_bps }}
 */
function getUnwrapFee(chainId, amount) {
  var chain = chains.get(chainId);
  if (!chain) throw new Error("unknown chainId: " + chainId);

  var tiers = chain.fee_unwrap_tiers;
  var bps;
  var tier;

  if (amount < tiers.small_threshold) {
    bps = tiers.small_bps;
    tier = "small";
  } else if (amount <= tiers.large_threshold) {
    bps = tiers.mid_bps;
    tier = "mid";
  } else {
    bps = tiers.large_bps;
    tier = "large";
  }

  var fee = _round((bps / 10000) * amount);
  var net = _round(amount - fee);
  return { fee: fee, net: net, tier: tier, rate_bps: bps };
}

/**
 * Get the current veCRV-style weight for a funder.
 * weight = amount × remaining_lock_days.
 * Returns 0 if funder not found or lock expired.
 */
function getLPWeight(funder, chainId) {
  var key = _funderKey(chainId, funder);
  var lock = funders.get(key);
  if (!lock) return 0;

  var now = _now();
  var remainingMs = lock.lock_expires_at - now;
  if (remainingMs <= 0) return 0;

  var remainingDays = remainingMs / (86400 * 1000);
  return _round(lock.amount * remainingDays);
}

/**
 * Get total HONE locked across all funders for this chain.
 */
function getTotalLocked(chainId) {
  var chain = chains.get(chainId);
  if (!chain) return 0;
  return chain.total_locked_hone;
}

/**
 * Get currently circulating wHONE on the destination chain
 * (i.e., wHONE that has been released from reserve).
 */
function getCirculatingWhone(chainId) {
  var chain = chains.get(chainId);
  if (!chain) return 0;
  return chain.circulating_whone;
}

/**
 * Get the current smoothing buffer balance for this chain (in HONE).
 * Capped at SMOOTHING_CAP_FRACTION of total locked liquidity.
 */
function getSmoothingBuffer(chainId) {
  return smoothingBuffers.get(chainId) || 0;
}

/**
 * Add to the smoothing buffer. Called by bridgeFeeDistributor when fees accrue.
 * Internally caps the buffer at 10% of total locked liquidity.
 *
 * @param {string} chainId
 * @param {number} amount — HONE to add to the buffer
 */
function _addToSmoothingBuffer(chainId, amount) {
  var chain = chains.get(chainId);
  if (!chain) return;
  var current = smoothingBuffers.get(chainId) || 0;
  var cap = _round(chain.total_locked_hone * SMOOTHING_CAP_FRACTION);
  var next = _round(current + amount);
  smoothingBuffers.set(chainId, Math.min(next, cap > 0 ? cap : next));
}

/**
 * Expose smoothing buffer add for bridgeFeeDistributor.
 */
function accrueSmoothingBuffer(chainId, amount) {
  _addToSmoothingBuffer(chainId, amount);
  return getSmoothingBuffer(chainId);
}

/**
 * Get all funders for a chain with their current weights.
 */
function getAllFunders(chainId) {
  var result = [];
  funders.forEach(function (lock, key) {
    if (lock.chain_id === chainId) {
      var weight = getLPWeight(lock.funder, chainId);
      result.push(Object.assign({}, lock, { current_weight: weight }));
    }
  });
  return result;
}

/**
 * Wipe all state. For tests only.
 */
function resetForTests() {
  chains.clear();
  funders.clear();
  withdrawalQueues.clear();
  smoothingBuffers.clear();
  volumeStats.clear();
}

module.exports = {
  // Constants (exported for tests and other modules)
  WHONE_SUPPLY_PER_CHAIN: WHONE_SUPPLY_PER_CHAIN,
  MIN_LOCK_DAYS: MIN_LOCK_DAYS,
  MAX_LOCK_DAYS: MAX_LOCK_DAYS,

  // Chain management
  registerChain: registerChain,
  getChain: getChain,
  getAllChains: getAllChains,

  // LP funding + withdrawal
  fundBridge: fundBridge,
  requestUnlock: requestUnlock,
  processUnlock: processUnlock,
  getAllFunders: getAllFunders,

  // Bridge operations
  wrap: wrap,
  unwrap: unwrap,

  // Fee queries
  getWrapFee: getWrapFee,
  getUnwrapFee: getUnwrapFee,

  // LP state
  getLPWeight: getLPWeight,
  getTotalLocked: getTotalLocked,
  getCirculatingWhone: getCirculatingWhone,

  // Smoothing buffer
  getSmoothingBuffer: getSmoothingBuffer,
  accrueSmoothingBuffer: accrueSmoothingBuffer,

  // Tests
  resetForTests: resetForTests,
};

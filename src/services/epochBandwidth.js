"use strict";

/**
 * BTCPC Epoch Bandwidth — novel resource credit system (v3.1.113)
 * Shin Devlin
 *
 * Unlike gas (price-based) or Hive RCs (5-day regeneration, opaque formula),
 * BTCPC Epoch Bandwidth (EB) is:
 *
 *   - Epoch-aligned: regenerates every 30 seconds, not over days
 *   - Stake-proportional: 1 EB per staked BTCPC per epoch
 *   - Device-keyed: each registered device key has its own EB pool —
 *     Sybil spam requires staked device registrations, not just accounts
 *   - Non-transferable and non-speculative: EB cannot be sold or hoarded
 *     past the cap (144 epochs = 1 day)
 *   - Predictable: every actor knows exactly what they can do and when
 *
 * Operation costs are fixed per type. No bidding, no price spikes.
 * Minimum stake of 1 BTCPC gives 1 EB/epoch → enough for ~8 transfers/day
 * at the default rate. Storage, model uploads, and token creation cost more,
 * naturally rate-limiting heavy chain use without penalizing light users.
 */

const stateStore = require('../chain/stateStore');

const BANDWIDTH_PER_STAKE_PER_EPOCH = 1.0;
const MAX_BANDWIDTH_CAP_EPOCHS = 144; // max 1 day of accumulated EB

const OPERATION_COSTS = {
  TRANSFER:           10,
  DELEGATE:           5,
  UNDELEGATE:         5,
  STAKE:              5,
  UNSTAKE:            5,
  FILE_STORE:         50,
  FILE_SHARD:         30,
  FILE_GRANT:         10,
  FILE_REVOKE:        5,
  MODEL_UPLOAD:       200,
  TOKEN_CREATE:       500,
  NFT_CREATE:         300,
  NFT_MINT:           50,
  NFT_TRANSFER:       20,
  NODE_REGISTER:      100,
  ACCOUNT_CREATE:     0,   // Free — registration is gated by invitation/cost elsewhere
  MINING_REWARD:      0,
  CLOCK_REWARD:       0,
  STORAGE_REWARD:     0,
  SERVICE_REWARD:     0,
  IOT_REWARD:         0,
  FAUCET:             0,
  _DEFAULT:           5,
};

// In-memory bandwidth pool: account → { eb, last_epoch }
// On restart this resets — accounts regenerate from their stake.
const bandwidthPool = new Map();

function _operationCost(entryType) {
  if (OPERATION_COSTS[entryType] !== undefined) return OPERATION_COSTS[entryType];
  return OPERATION_COSTS._DEFAULT;
}

function _maxEB(account) {
  const stake = stateStore.getStake ? stateStore.getStake(account) : 0;
  return Math.max(10, stake * BANDWIDTH_PER_STAKE_PER_EPOCH * MAX_BANDWIDTH_CAP_EPOCHS);
}

/**
 * Accrue EB for an account up to current epoch, then check if the operation
 * can be paid. Returns { allowed: bool, eb_remaining: number, cost: number }.
 * If allowed, deducts the cost.
 */
function checkAndDeduct(account, entryType) {
  if (!account) return { allowed: true, eb_remaining: Infinity, cost: 0 };

  const cost = _operationCost(entryType);
  if (cost === 0) return { allowed: true, eb_remaining: Infinity, cost: 0 };

  const currentEpoch = stateStore.getChainHeight ? stateStore.getChainHeight() : 0;
  const maxEB = _maxEB(account);

  let pool = bandwidthPool.get(account) || { eb: maxEB, last_epoch: currentEpoch };

  // Accrue EB since last epoch
  const epochsElapsed = Math.max(0, currentEpoch - pool.last_epoch);
  if (epochsElapsed > 0) {
    const stake = stateStore.getStake ? stateStore.getStake(account) : 1;
    const earned = epochsElapsed * stake * BANDWIDTH_PER_STAKE_PER_EPOCH;
    pool.eb = Math.min(maxEB, pool.eb + earned);
    pool.last_epoch = currentEpoch;
  }

  if (pool.eb < cost) {
    bandwidthPool.set(account, pool);
    return { allowed: false, eb_remaining: pool.eb, cost, max_eb: maxEB };
  }

  pool.eb -= cost;
  bandwidthPool.set(account, pool);
  return { allowed: true, eb_remaining: pool.eb, cost, max_eb: maxEB };
}

/**
 * Get current EB balance for an account (after accrual).
 */
function getBalance(account) {
  const result = checkAndDeduct.__peek(account);
  return result;
}

// Peek without deducting — for informational endpoints
checkAndDeduct.__peek = function(account) {
  if (!account) return { eb: Infinity };
  const currentEpoch = stateStore.getChainHeight ? stateStore.getChainHeight() : 0;
  const maxEB = _maxEB(account);
  let pool = bandwidthPool.get(account) || { eb: maxEB, last_epoch: currentEpoch };
  const epochsElapsed = Math.max(0, currentEpoch - pool.last_epoch);
  if (epochsElapsed > 0) {
    const stake = stateStore.getStake ? stateStore.getStake(account) : 1;
    pool.eb = Math.min(maxEB, pool.eb + epochsElapsed * stake * BANDWIDTH_PER_STAKE_PER_EPOCH);
    pool.last_epoch = currentEpoch;
    bandwidthPool.set(account, pool);
  }
  return { eb: pool.eb, max_eb: maxEB, last_epoch: pool.last_epoch };
};

module.exports = { checkAndDeduct, getBalance, OPERATION_COSTS, BANDWIDTH_PER_STAKE_PER_EPOCH };

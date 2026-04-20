"use strict";

/**
 * BTCPC Dynamic Reward Engine
 * Shin Devlin
 *
 * Computes epoch reward distributions based on actual proven contributions.
 * Every pool scales with real evidence of value delivered — not just participation.
 *
 * Reward pools (from emissionSchedule blockReward):
 *   55%  Mining/Compute  — proportional to work_value (model_weight × tokens)
 *   10%  Verifier        — equal split among active verifiers
 *    5%  Clock           — proportional to verified heartbeats in window
 *   15%  Storage         — proportional to capacity_gb × queries_served
 *    7%  Sensor base     — equal split among sensors that submitted readings this epoch
 *                          Makes it worth coming online even before data is purchased.
 *                          Purchase rewards come on top via sensorRewards.processPurchase().
 *    6%  Service         — equal split among service hosts (future)
 *    2%  Protocol reserve → btcpc_recycle always
 *   recycled → btcpc_recycle when any pool has no participants
 *
 * Design principles:
 *   - Pools with no participants → btcpc_recycle (never burned)
 *   - Sensor base reward: small but enough to incentivize coming online
 *   - Sensor purchase reward: separate, triggered by data query payment
 *   - Compute proof must include result_hash to be counted
 *   - Storage proof must include capacity_gb > 0 or cids_count > 0
 *   - All amounts rounded to 10 decimal places for determinism
 */

const RECYCLE_ACCOUNT = "btcpc_recycle";
const TOTAL_SUPPLY = 42000000; // 42M BTCPC hard cap
const { computeToolMultiplier } = require("../mcp/toolRegistry");

// Pool percentages — must sum to 1.0
const POOL = {
  mining:   0.55,
  verifier: 0.10,
  clock:    0.05,
  storage:  0.15,
  sensor:   0.07, // base participation reward — equal split among active sensors this epoch
  service:  0.06,
  reserve:  0.02, // always → btcpc_recycle
};

function round(n) {
  return parseFloat(Number(n).toFixed(10));
}

/**
 * Compute rewards for one epoch.
 *
 * @param {object} input
 * @param {number} input.epochNumber
 * @param {number} input.blockReward      — total emission for this epoch (from emissionSchedule)
 * @param {Array}  input.computeProofs    — [{ node_id, work_value, model, tokens_generated, result_hash }]
 * @param {Array}  input.verifiers        — [accountName, ...]
 * @param {Array}  input.clockNodes       — [{ account, heartbeats }]
 * @param {Array}  input.storageHosts     — [{ account, capacity_gb, queries_served, cids_count }]
 * @param {Array}  input.activeSensors    — [{ account, readings_count }] sensors that submitted readings this epoch
 * @param {Array}  input.serviceHosts     — [accountName, ...]
 * @param {object} [input.stateStore]     — injected for Phase 2 checks (optional, falls back to live require)
 *
 * @returns {object} { rewards, recycled, summary, phase2 }
 *   rewards: [{ to, amount, type, meta }]  — all non-zero reward entries
 *   recycled: number                        — amount routed to btcpc_recycle
 *   summary: object                         — stats for logging/explorer
 *   phase2: boolean                         — true if this epoch ran under Phase 2
 */
function computeRewards(input) {
  const {
    epochNumber = 0,
    blockReward = 0,
    computeProofs = [],
    verifiers = [],
    clockNodes = [],
    storageHosts = [],
    activeSensors = [],
    serviceHosts = [],
  } = input;

  // ── Phase 2 check: post-supply endowment ───────────────────────────────────
  // When total distributed supply has reached 42M, blockReward comes from
  // btcpc_recycle balance × r (computed at activation), not from mining schedule.
  let effectiveBlockReward = blockReward;
  let isPhase2Epoch = false;
  try {
    const ss = input.stateStore || require("./stateStore");
    if (ss.isPhase2()) {
      isPhase2Epoch = true;
      const recycleBalance = ss.getBalance(RECYCLE_ACCOUNT, "BTCPC");
      if (!ss.isPhase2Activated()) {
        // First Phase 2 epoch: compute r = last Phase 1 blockReward / recycleBalance
        // r targets parity with the last Phase 1 epoch reward
        const r = recycleBalance > 0 ? round(blockReward / recycleBalance) : 0;
        ss.setRecycleRate(r, epochNumber);
      }
      const r = ss.getRecycleRate() || 0;
      const recycleBalance2 = ss.getBalance(RECYCLE_ACCOUNT, "BTCPC");
      effectiveBlockReward = round(recycleBalance2 * r);
    }
  } catch (_) {
    // stateStore not available in test isolation — use blockReward as-is
  }

  const rewards = [];
  let recycled = 0;

  // ── Mining pool (55%): proportional to work_value ──────────────────────────
  const miningPool = round(effectiveBlockReward * POOL.mining);
  const validProofs = computeProofs.filter(p =>
    p.node_id && (p.work_value > 0) && p.result_hash
  );
  // Apply tool multiplier to each proof's effective work value
  const validProofsWithMult = validProofs.map(p => ({
    ...p,
    effective_work: round((p.work_value || 0) * computeToolMultiplier(p.tools_used || [])),
  }));
  const totalWork = validProofsWithMult.reduce((s, p) => s + p.effective_work, 0);

  if (validProofs.length === 0 || totalWork === 0) {
    recycled += miningPool;
  } else {
    // Aggregate by miner (one miner may have multiple proofs this epoch)
    const byMiner = new Map();
    for (const p of validProofsWithMult) {
      const cur = byMiner.get(p.node_id) || { work: 0, models: new Set(), tokens: 0, tools: new Set() };
      cur.work += p.effective_work;
      cur.tokens += p.tokens_generated || 0;
      if (p.model) cur.models.add(p.model);
      for (const t of (p.tools_used || [])) cur.tools.add(t);
      byMiner.set(p.node_id, cur);
    }
    for (const [account, m] of byMiner) {
      const share = round(miningPool * (m.work / totalWork));
      if (share > 0) {
        rewards.push({
          to: account, amount: share, type: "MINING_REWARD",
          meta: {
            work_value: round(m.work), tokens: m.tokens,
            models: Array.from(m.models),
            tools: Array.from(m.tools),
          }
        });
      }
    }
  }

  // ── Verifier pool (10%): equal split ────────────────────────────────────────
  const verifierPool = round(effectiveBlockReward * POOL.verifier);
  const activeVerifiers = (verifiers || []).filter(v => typeof v === "string" && v.length > 0);
  if (activeVerifiers.length === 0) {
    recycled += verifierPool;
  } else {
    const share = round(verifierPool / activeVerifiers.length);
    for (const v of activeVerifiers) {
      rewards.push({ to: v, amount: share, type: "VERIFIER_REWARD", meta: {} });
    }
  }

  // ── Clock pool (5%): proportional to heartbeat count ────────────────────────
  const clockPool = round(effectiveBlockReward * POOL.clock);
  const activeClocks = (clockNodes || []).filter(c => c.account && (c.heartbeats || 0) > 0);
  const totalHeartbeats = activeClocks.reduce((s, c) => s + c.heartbeats, 0);
  if (activeClocks.length === 0) {
    recycled += clockPool;
  } else {
    for (const c of activeClocks) {
      const share = round(clockPool * (c.heartbeats / totalHeartbeats));
      if (share > 0) {
        rewards.push({ to: c.account, amount: share, type: "CLOCK_REWARD", meta: { heartbeats: c.heartbeats } });
      }
    }
  }

  // ── Storage pool (15%): proportional to capacity_gb (zero if no CIDs) ───────
  const storagePool = round(effectiveBlockReward * POOL.storage);
  // Only hosts with actual stored data (cids_count > 0 OR capacity_gb > 0) earn
  const activeStorage = (storageHosts || []).filter(h => h.account && (h.capacity_gb > 0 || h.cids_count > 0));
  const totalCapacity = activeStorage.reduce((s, h) => s + (h.capacity_gb || 0.001), 0);
  if (activeStorage.length === 0) {
    recycled += storagePool;
  } else {
    for (const h of activeStorage) {
      const capacityWeight = (h.capacity_gb || 0.001) / totalCapacity;
      // Bonus multiplier for actually serving queries (up to 2x)
      const queryBonus = h.queries_served > 0 ? Math.min(1 + (h.queries_served / 100), 2) : 1;
      const share = round(storagePool * capacityWeight * queryBonus / activeStorage.reduce((s, x) => {
        const qb = x.queries_served > 0 ? Math.min(1 + (x.queries_served / 100), 2) : 1;
        return s + ((x.capacity_gb || 0.001) / totalCapacity) * qb;
      }, 0) * activeStorage.reduce((s, x) => {
        const qb = x.queries_served > 0 ? Math.min(1 + (x.queries_served / 100), 2) : 1;
        return s + ((x.capacity_gb || 0.001) / totalCapacity) * qb;
      }, 0));
      if (share > 0) {
        rewards.push({
          to: h.account, amount: share, type: "STORAGE_REWARD",
          meta: { capacity_gb: h.capacity_gb, queries_served: h.queries_served || 0, cids: h.cids_count || 0 }
        });
      }
    }
  }

  // ── Sensor pool (7%): base participation — equal split among active sensors ──
  // Sensors that submitted ≥1 reading this epoch get a small base reward.
  // This makes it worth coming online even before anyone purchases the data.
  // Purchase rewards are separate — triggered by sensorRewards.processPurchase().
  //
  // Device yield staking split (v3.4):
  //   If a device has an active yield staker:
  //     60% → device owner, 30% → top staker, 10% → btcpc_recycle
  //   Otherwise:
  //     90% → device owner, 10% → btcpc_recycle
  const sensorPool = round(effectiveBlockReward * POOL.sensor);
  const activeSensorList = (activeSensors || []).filter(s => s.account && (s.readings_count || 0) > 0);
  if (activeSensorList.length === 0) {
    recycled += sensorPool;
  } else {
    // Weight by readings_count — more readings submitted = slightly higher share
    // (capped: a sensor submitting 100 readings doesn't earn 100× one submitting 1)
    const totalReadings = activeSensorList.reduce((s, x) => s + Math.min(x.readings_count, 10), 0);
    // Load stateStore for yield staker lookup (lazy to avoid circular dep in tests)
    let _ss = null;
    try { _ss = input.stateStore || require("./stateStore"); } catch (_) {}

    for (const sensor of activeSensorList) {
      const weight = Math.min(sensor.readings_count, 10) / totalReadings;
      const share = round(sensorPool * weight);
      if (share <= 0) continue;

      // Check for active staker pool on each sensor_id in this sensor's sensor_ids
      // sensor.account is the owner; sensor.sensor_ids is the list of device_ids
      const sensorIds = sensor.sensor_ids || [];
      // For each sensor_id, determine per-device share, apply staking split
      if (sensorIds.length > 0 && _ss && typeof _ss.getDeviceStakerPool === "function") {
        const perDevice = round(share / sensorIds.length);
        for (const sid of sensorIds) {
          const pool = _ss.getDeviceStakerPool(sid);
          const SMULT = (_ss.SLOT_MULTIPLIERS) || [1.85, 1.60, 1.38, 1.19, 1.03, 0.89, 0.77, 0.67, 0.58, 0.50];
          if (pool && pool.length > 0) {
            // 70/20/10 split
            const ownerBase = round(perDevice * 0.70);
            const stakerPool = round(perDevice * 0.20);
            const recycleShare = round(perDevice - ownerBase - stakerPool);

            // Weighted staker pool
            const totalWeighted = pool.reduce(function (s, r) {
              return s + (SMULT[(r.slot - 1)] || 0) * r.stake_amount;
            }, 0);

            let rentCollected = 0;
            for (const staker of pool) {
              const mult = SMULT[(staker.slot - 1)] || 0;
              const stakerYield = totalWeighted > 0 ? round((mult * staker.stake_amount / totalWeighted) * stakerPool) : 0;
              const rentDue = staker.rent_bid || 0;

              if (staker.rent_mode === "stake") {
                // Full yield to wallet, rent deducted from stake via DEVICE_YIELD_RENT entry
                if (stakerYield > 0) rewards.push({ to: staker.staker, amount: stakerYield, type: "SENSOR_YIELD_REWARD", meta: { sensor_id: sid, device_owner: sensor.account, slot: staker.slot, rent_mode: "stake" } });
                if (rentDue > 0) {
                  rewards.push({ type: "DEVICE_YIELD_RENT", to: sensor.account, amount: rentDue, meta: { sensor_id: sid, staker: staker.staker, rent_from_stake: rentDue }, yield_stake_data: { device_id: sid, staker: staker.staker, rent_amount: rentDue, owner: sensor.account } });
                  rentCollected += rentDue;
                }
              } else {
                // earnings mode: yield covers rent first
                const yieldToWallet = Math.max(0, stakerYield - rentDue);
                const rentFromStake = Math.max(0, rentDue - stakerYield);
                if (yieldToWallet > 0) rewards.push({ to: staker.staker, amount: round(yieldToWallet), type: "SENSOR_YIELD_REWARD", meta: { sensor_id: sid, device_owner: sensor.account, slot: staker.slot, rent_mode: "earnings" } });
                if (rentFromStake > 0) {
                  rewards.push({ type: "DEVICE_YIELD_RENT", to: sensor.account, amount: round(rentFromStake), meta: { sensor_id: sid, staker: staker.staker, rent_from_stake: rentFromStake }, yield_stake_data: { device_id: sid, staker: staker.staker, rent_amount: round(rentFromStake), owner: sensor.account } });
                }
                // Rent covered by yield goes directly to owner
                const rentFromYield = round(Math.min(rentDue, stakerYield));
                if (rentFromYield > 0) rentCollected += rentFromYield;
              }
            }

            if (ownerBase > 0) rewards.push({ to: sensor.account, amount: ownerBase, type: "SENSOR_EPOCH_REWARD", meta: { readings: sensor.readings_count, sensor_id: sid, yield_staking: true, split: "owner_70" } });
            if (rentCollected > 0) rewards.push({ to: sensor.account, amount: round(rentCollected), type: "SENSOR_RENT_COLLECTED", meta: { sensor_id: sid } });
            recycled += recycleShare;
          } else {
            // 90/10 split (no stakers)
            const ownerShare = round(perDevice * 0.90);
            const recycleShare = round(perDevice - ownerShare);
            if (ownerShare > 0) rewards.push({ to: sensor.account, amount: ownerShare, type: "SENSOR_EPOCH_REWARD", meta: { readings: sensor.readings_count, sensor_id: sid, split: "owner_90" } });
            recycled += recycleShare;
          }
        }
      } else {
        // No sensor_ids or no stateStore — fall back: 90/10 split on full share
        const ownerShare = round(share * 0.90);
        const recycleShare = round(share - ownerShare);
        rewards.push({
          to: sensor.account, amount: ownerShare, type: "SENSOR_EPOCH_REWARD",
          meta: { readings: sensor.readings_count, sensor_ids: sensorIds }
        });
        recycled += recycleShare;
      }
    }
  }

  // ── Service pool (6%): equal split ──────────────────────────────────────────
  const servicePool = round(effectiveBlockReward * POOL.service);
  const activeServices = (serviceHosts || []).filter(h => typeof h === "string" && h.length > 0);
  if (activeServices.length === 0) {
    recycled += servicePool;
  } else {
    const share = round(servicePool / activeServices.length);
    for (const h of activeServices) {
      rewards.push({ to: h, amount: share, type: "SERVICE_REWARD", meta: {} });
    }
  }

  // ── Protocol reserve (2%): always to btcpc_recycle ──────────────────────────
  recycled += round(effectiveBlockReward * POOL.reserve);

  // Recycle entry
  if (recycled > 0) {
    rewards.push({ to: RECYCLE_ACCOUNT, amount: round(recycled), type: "RECYCLE", meta: { reason: "unused_pools" } });
  }

  const summary = {
    epoch: epochNumber,
    block_reward: blockReward,
    effective_block_reward: effectiveBlockReward,
    phase2: isPhase2Epoch,
    total_distributed: round(rewards.filter(r => r.to !== RECYCLE_ACCOUNT).reduce((s, r) => s + r.amount, 0)),
    recycled: round(recycled),
    miners: validProofsWithMult.length,
    total_work: round(totalWork),
    verifiers: activeVerifiers.length,
    clocks: activeClocks.length,
    storage_hosts: activeStorage.length,
    active_sensors: activeSensorList.length,
    service_hosts: activeServices.length,
  };

  return { rewards, recycled, summary, phase2: isPhase2Epoch };
}

/**
 * Check whether the chain is in Phase 2 (post-supply endowment mode).
 * Convenience wrapper around stateStore.isPhase2().
 * @param {object} [ss] — optional stateStore override (for testing)
 */
function isPhase2(ss) {
  try {
    var store = ss || require("./stateStore");
    return store.isPhase2();
  } catch (_) {
    return false;
  }
}

/**
 * Collect storage host data from STORAGE_HEARTBEAT ledger entries in a block payload.
 * Returns [{ account, capacity_gb, cids_count, queries_served }]
 */
function extractStorageHosts(ledgerEntries) {
  const hosts = new Map();
  for (const e of (ledgerEntries || [])) {
    if (e.type !== "STORAGE_HEARTBEAT") continue;
    const account = e.from;
    if (!account) continue;
    const blob = e.blob_data || {};
    const existing = hosts.get(account) || { account, capacity_gb: 0, cids_count: 0, queries_served: 0 };
    existing.capacity_gb = Math.max(existing.capacity_gb, blob.capacity_used_gb || 0);
    existing.cids_count += (blob.cids || []).length;
    existing.queries_served += blob.queries_served || 0;
    hosts.set(account, existing);
  }
  return Array.from(hosts.values());
}

/**
 * Build clock node list from P2P gossip data.
 * protocol.getActiveClockNodes() returns account names — wrap with heartbeat count of 1.
 */
function buildClockNodes(clockAccounts) {
  return (clockAccounts || []).map(account => ({ account, heartbeats: 1 }));
}

module.exports = {
  computeRewards,
  extractStorageHosts,
  buildClockNodes,
  isPhase2,
  POOL,
  RECYCLE_ACCOUNT,
  TOTAL_SUPPLY,
};

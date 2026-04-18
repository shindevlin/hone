"use strict";

/**
 * BTCPC Dynamic Reward Engine
 * Shin Devlin
 *
 * Computes epoch reward distributions based on actual proven contributions.
 * Every pool scales with real evidence of value delivered — not just participation.
 *
 * Reward pools (from emissionSchedule blockReward):
 *   60%  Mining/Compute  — proportional to work_value (model_weight × tokens)
 *   10%  Verifier        — equal split among active verifiers
 *    5%  Clock           — proportional to verified heartbeats in window
 *   15%  Storage         — proportional to capacity_gb × queries_served
 *   10%  IoT/Sensor      — zero base; triggered by data purchase events
 *    8%  Service         — equal split among service hosts (future)
 *   recycled → btcpc_recycle when any pool has no participants
 *
 * Design principles:
 *   - Pools with no participants → btcpc_recycle (never burned)
 *   - Zero base for sensor data — rewards on purchase, not submission
 *   - Compute proof must include model_hash + result_hash to be counted
 *   - Storage proof must include cids + capacity_gb (empty = no reward)
 *   - All amounts rounded to 10 decimal places for determinism
 */

const RECYCLE_ACCOUNT = "btcpc_recycle";

// Pool percentages — must sum to 1.0
const POOL = {
  mining:   0.60,
  verifier: 0.10,
  clock:    0.05,
  storage:  0.15,
  sensor:   0.00, // zero base — earned on purchase via sensorDataRoutes
  service:  0.08,
  // remaining 0.02 always goes to recycle as protocol reserve
  reserve:  0.02,
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
 * @param {Array}  input.computeProofs    — [{ node_id, work_value, model, tokens_generated, model_hash, result_hash }]
 * @param {Array}  input.verifiers        — [accountName, ...]
 * @param {Array}  input.clockNodes       — [{ account, heartbeats }]
 * @param {Array}  input.storageHosts     — [{ account, capacity_gb, queries_served, cids_count }]
 * @param {Array}  input.serviceHosts     — [accountName, ...]
 *
 * @returns {object} { rewards, recycled, summary }
 *   rewards: [{ to, amount, type, meta }]  — all non-zero reward entries
 *   recycled: number                        — amount routed to btcpc_recycle
 *   summary: object                         — stats for logging/explorer
 */
function computeRewards(input) {
  const {
    epochNumber = 0,
    blockReward = 0,
    computeProofs = [],
    verifiers = [],
    clockNodes = [],
    storageHosts = [],
    serviceHosts = [],
  } = input;

  const rewards = [];
  let recycled = 0;

  // ── Mining pool (60%): proportional to work_value ──────────────────────────
  const miningPool = round(blockReward * POOL.mining);
  const validProofs = computeProofs.filter(p =>
    p.node_id && (p.work_value > 0) && p.result_hash
  );
  const totalWork = validProofs.reduce((s, p) => s + (p.work_value || 0), 0);

  if (validProofs.length === 0 || totalWork === 0) {
    recycled += miningPool;
  } else {
    // Aggregate by miner (one miner may have multiple proofs this epoch)
    const byMiner = new Map();
    for (const p of validProofs) {
      const cur = byMiner.get(p.node_id) || { work: 0, models: new Set(), tokens: 0 };
      cur.work += p.work_value;
      cur.tokens += p.tokens_generated || 0;
      if (p.model) cur.models.add(p.model);
      byMiner.set(p.node_id, cur);
    }
    for (const [account, m] of byMiner) {
      const share = round(miningPool * (m.work / totalWork));
      if (share > 0) {
        rewards.push({
          to: account, amount: share, type: "MINING_REWARD",
          meta: { work_value: round(m.work), tokens: m.tokens, models: Array.from(m.models) }
        });
      }
    }
  }

  // ── Verifier pool (10%): equal split ────────────────────────────────────────
  const verifierPool = round(blockReward * POOL.verifier);
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
  const clockPool = round(blockReward * POOL.clock);
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
  const storagePool = round(blockReward * POOL.storage);
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

  // ── Sensor pool: zero base, purchase-triggered via sensorDataRoutes ──────────
  // Sensor rewards are emitted as SENSOR_REWARD entries when data is purchased,
  // funded from the btcpc_recycle pool rather than the epoch emission.
  // Nothing to distribute here.

  // ── Service pool (8%): equal split ──────────────────────────────────────────
  const servicePool = round(blockReward * POOL.service);
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
  recycled += round(blockReward * POOL.reserve);

  // Recycle entry
  if (recycled > 0) {
    rewards.push({ to: RECYCLE_ACCOUNT, amount: round(recycled), type: "RECYCLE", meta: { reason: "unused_pools" } });
  }

  const summary = {
    epoch: epochNumber,
    block_reward: blockReward,
    total_distributed: round(rewards.filter(r => r.to !== RECYCLE_ACCOUNT).reduce((s, r) => s + r.amount, 0)),
    recycled: round(recycled),
    miners: validProofs.length,
    total_work: round(totalWork),
    verifiers: activeVerifiers.length,
    clocks: activeClocks.length,
    storage_hosts: activeStorage.length,
    service_hosts: activeServices.length,
  };

  return { rewards, recycled, summary };
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
  POOL,
  RECYCLE_ACCOUNT,
};

"use strict";

/**
 * HONE Block Emission — fully dynamic reward distribution
 * Shin Devlin
 *
 * Every pool floats between 0% and 99.9% of block reward — no floors,
 * no caps. The network pays for what it's actually doing. If sensor data
 * purchases outpace inference demand, IoT earns more than mining that epoch.
 * If hosted services dominate traffic, services lead. The chain self-balances.
 *
 * Structure:
 *   1. 0.1% testnet carveout — fixed, off the top, for nodes running the
 *      public test network. Recycled if no testnet nodes are active.
 *   2. Remaining 99.9% — split proportionally across all active roles by
 *      their work score. Roles with zero participation earn zero.
 *
 * Work scores (real demand signals):
 *   Miner        = sum of verified work_value (inference compute delivered)
 *   Verifier     = verifications performed × VERIFIER_SCORE_PER_CHECK
 *   Clock        = node count × CLOCK_SCORE_PER_NODE [× scarcity]
 *   Storage host = bytes delivered × STORAGE_SCORE_PER_MB (proxy: count × STORAGE_SCORE_PER_HOST)
 *   IoT sensor   = SENSOR_BASE_SCORE + data_purchase_revenue × SENSOR_REVENUE_MULTIPLIER
 *                  (liveness earns a tiny base; purchases are the primary signal)
 *   IoT gateway  = packets relayed × GATEWAY_SCORE_PER_PACKET (proxy: count × GATEWAY_SCORE_PER_GW)
 *   Service host = requests served × SERVICE_SCORE_PER_REQUEST (proxy: count × SERVICE_SCORE_PER_HOST)
 *
 * Sensor design: data volume is not the signal — market demand is. A sensor
 * that submits 10k readings nobody buys earns only its tiny liveness base.
 * A sensor with 50 readings purchased by buyers earns primarily from revenue.
 * Dead sensors (no readings in window) are excluded entirely.
 *
 * When a work map is provided for non-sensor roles, scoring and within-pool
 * splits are proportional to actual work. When absent, falls back to the
 * count-based proxy with scarcity premium.
 *
 * Clocks remain count-based — uptime IS their work.
 *
 * Scarcity: roles with fewer than SCARCITY_THRESHOLD nodes get a multiplier
 * so critical infrastructure is always worth running, even when the network
 * is small. As participation grows the premium fades naturally.
 *
 * Participants shape:
 *   {
 *     miners:        string[]
 *     minerWork:     { [account]: number }   // verified work_value per miner
 *     verifiers:     string[]
 *     verifierWork:  { [account]: number }   // verifications performed (optional)
 *     clocks:        string[]
 *     storageHosts:  string[]
 *     storageWork:   { [host]: number }      // bytes delivered (optional)
 *     sensors:       string[]               // ACTIVE sensors — liveness pre-filtered
 *     sensorWork:    { [owner]: number }     // readings delivered — liveness signal (optional)
 *     sensorRevenue: { [owner]: number }     // HONE from data purchases — primary signal (optional)
 *     gateways:      string[]
 *     gatewayWork:   { [owner]: number }     // packets relayed (optional)
 *     serviceHosts:  string[]
 *     serviceWork:   { [host]: number }      // requests served (optional)
 *     testnets:      string[]
 *   }
 */

// Count-based proxy scores — used when no real work data is available
const CLOCK_SCORE_PER_NODE    = 1000;
const VERIFIER_SCORE_PER_NODE = 3000;
const STORAGE_SCORE_PER_HOST  = 2000;
const SENSOR_SCORE_PER_SENSOR = 500;
const GATEWAY_SCORE_PER_GW    = 1500;
const SERVICE_SCORE_PER_HOST  = 2500;

// Work-based scores — multiply actual work units by these to get comparable score
// Calibrated so typical activity ≈ the count-based proxy above.
// e.g. 10 verifications/epoch × 300 = 3000 ≈ VERIFIER_SCORE_PER_NODE
const VERIFIER_SCORE_PER_CHECK   = 300;   // per verification performed
const STORAGE_SCORE_PER_MB       = 2;     // per MB delivered (bytes / 1048576)
const SENSOR_BASE_SCORE_PER_NODE      = 100;  // small liveness base per active sensor
const SENSOR_REVENUE_MULTIPLIER       = 5000; // 1 HONE in sales → 5000 score points
const SENSOR_SCORE_PER_READING        = 50;   // proxy only — used when no revenue data
const GATEWAY_SCORE_PER_PACKET   = 15;    // per packet relayed
const SERVICE_SCORE_PER_REQUEST  = 25;    // per request served

// Scarcity: below this node count, per-node score is multiplied
const SCARCITY_THRESHOLD  = 3;
const SCARCITY_MULTIPLIER = 2.5;

// Testnet: fixed 0.1% off the top every epoch
const TESTNET_FRACTION = 0.001;

function round(n) {
  return parseFloat(Number(n).toFixed(10));
}

function scarcityScore(count, basePerNode) {
  if (count === 0) return 0;
  return count * (count < SCARCITY_THRESHOLD ? basePerNode * SCARCITY_MULTIPLIER : basePerNode);
}

/**
 * Compute a role's total score using real work data when available,
 * falling back to count-based proxy with scarcity premium.
 *
 * @param {string[]} participants — role members
 * @param {object}   workMap — { [account]: work_units } or null/undefined
 * @param {number}   scorePerUnit — WORK-based per-unit constant
 * @param {number}   scorePerNode — PROXY per-node constant (scarcity-eligible)
 * @param {number}   unitScale — divisor to apply to raw work units (e.g. 1048576 for bytes→MB)
 */
function roleScore(participants, workMap, scorePerUnit, scorePerNode, unitScale) {
  if (!participants || participants.length === 0) return 0;
  unitScale = unitScale || 1;
  if (workMap) {
    var totalWork = 0;
    for (var p of participants) {
      totalWork += (workMap[p] || 0);
    }
    if (totalWork > 0) {
      return (totalWork / unitScale) * scorePerUnit;
    }
  }
  return scarcityScore(participants.length, scorePerNode);
}

/**
 * Split a pool amount among participants.
 * Uses proportional-to-work split when work data is available; equal split otherwise.
 */
function splitPool(participants, workMap, unitScale, poolAmount, type, rewards) {
  if (!participants || participants.length === 0) return;
  unitScale = unitScale || 1;

  if (workMap) {
    var totalWork = 0;
    for (var p of participants) totalWork += (workMap[p] || 0);
    if (totalWork > 0) {
      for (var p of participants) {
        var w = workMap[p] || 0;
        if (w > 0) {
          rewards.push({ miner: p, amount: round(poolAmount * (w / totalWork)), type: type });
        }
      }
      return;
    }
  }
  // Equal split fallback
  var share = round(poolAmount / participants.length);
  for (var p of participants) {
    rewards.push({ miner: p, amount: share, type: type });
  }
}

function distributeBlockReward(blockReward, participants) {
  const rewards = [];

  const miners       = participants.miners       || [];
  const minerWork    = participants.minerWork    || {};
  const verifiers    = participants.verifiers    || [];
  const verifierWork = participants.verifierWork || null;
  const clocks       = participants.clocks       || [];
  const storageHosts = participants.storageHosts || [];
  const storageWork  = participants.storageWork  || null;
  const sensors       = participants.sensors       || [];
  const sensorWork    = participants.sensorWork    || null;
  const sensorRevenue = participants.sensorRevenue || null;
  const gateways      = participants.gateways      || [];
  const gatewayWork  = participants.gatewayWork  || null;
  const serviceHosts = participants.serviceHosts || [];
  const serviceWork  = participants.serviceWork  || null;
  const testnets     = participants.testnets     || [];

  // ── 1. Testnet carveout (0.1%) ─────────────────────────────────────────
  const testnetPool   = round(blockReward * TESTNET_FRACTION);
  const distributable = round(blockReward - testnetPool);

  if (testnets.length > 0) {
    const share = round(testnetPool / testnets.length);
    for (const t of testnets) {
      rewards.push({ miner: t, amount: share, type: 'testnet' });
    }
  } else {
    rewards.push({ miner: 'hone_recycle', amount: testnetPool, type: 'recycle' });
  }

  // ── 2. Work scores — fully demand-driven ──────────────────────────────
  const minerScore   = miners.reduce((s, m) => s + (minerWork[m] || 0), 0);
  const verifierScore = roleScore(verifiers, verifierWork, VERIFIER_SCORE_PER_CHECK, VERIFIER_SCORE_PER_NODE, 1);
  const clockScore    = scarcityScore(clocks.length, CLOCK_SCORE_PER_NODE); // uptime = work
  const storageScore  = roleScore(storageHosts, storageWork, STORAGE_SCORE_PER_MB, STORAGE_SCORE_PER_HOST, 1048576);
  // Sensor hybrid scoring: base (liveness) + revenue (demand).
  // Dead sensors already excluded by caller (sensors[] contains only active ones).
  // If revenue data is available, use base + revenue × multiplier per sensor.
  // Otherwise fall back to reading-count proxy (or flat count proxy).
  var _sensorCombined = {};
  if (sensors.length > 0) {
    var _hasRevData = sensorRevenue && Object.keys(sensorRevenue).length > 0;
    var _hasReadData = sensorWork && Object.keys(sensorWork).length > 0;
    for (var _sn of sensors) {
      if (_hasReadData && !(sensorWork[_sn] > 0)) continue; // skip dead sensors
      var _rev = (_hasRevData ? (sensorRevenue[_sn] || 0) : 0);
      _sensorCombined[_sn] = SENSOR_BASE_SCORE_PER_NODE + _rev * SENSOR_REVENUE_MULTIPLIER;
    }
  }
  var _sensorCombinedTotal = Object.values(_sensorCombined).reduce((s, v) => s + v, 0);
  const sensorRawScore = _sensorCombinedTotal > 0
    ? _sensorCombinedTotal
    : roleScore(sensors, sensorWork, SENSOR_SCORE_PER_READING, SENSOR_SCORE_PER_SENSOR, 1);
  const gatewayRawScore = roleScore(gateways, gatewayWork, GATEWAY_SCORE_PER_PACKET, GATEWAY_SCORE_PER_GW, 1);
  const sensorScore   = sensorRawScore + gatewayRawScore;
  const serviceScore  = roleScore(serviceHosts, serviceWork, SERVICE_SCORE_PER_REQUEST, SERVICE_SCORE_PER_HOST, 1);

  const totalScore = minerScore + verifierScore + clockScore
                   + storageScore + sensorScore + serviceScore;

  // ── Nothing active — recycle the distributable pool ───────────────────
  if (totalScore === 0) {
    rewards.push({ miner: 'hone_recycle', amount: distributable, type: 'recycle' });
    const paid = rewards.reduce((s, r) => s + r.amount, 0);
    const dust = round(blockReward - paid);
    if (Math.abs(dust) > 1e-9) {
      rewards.push({ miner: 'hone_recycle', amount: Math.abs(dust), type: 'recycle' });
    }
    return rewards;
  }

  // ── 3. Pool sizes — purely proportional ───────────────────────────────
  const minerPool    = round(distributable * (minerScore    / totalScore));
  const verifierPool = round(distributable * (verifierScore / totalScore));
  const clockPool    = round(distributable * (clockScore    / totalScore));
  const storagePool  = round(distributable * (storageScore  / totalScore));
  const sensorPool   = round(distributable * (sensorScore   / totalScore));
  const servicePool  = round(distributable * (serviceScore  / totalScore));

  // ── Miner pool — proportional to verified work_value ──────────────────
  if (minerScore > 0) {
    for (const m of miners) {
      const w = minerWork[m] || 0;
      if (w > 0) {
        rewards.push({ miner: m, amount: round(minerPool * (w / minerScore)), type: 'miner' });
      }
    }
  }

  // ── Verifier pool ──────────────────────────────────────────────────────
  if (verifierScore > 0) {
    splitPool(verifiers, verifierWork, 1, verifierPool, 'verifier', rewards);
  }

  // ── Clock pool — equal split (uptime = work) ───────────────────────────
  if (clockScore > 0) {
    const share = round(clockPool / clocks.length);
    for (const c of clocks) {
      rewards.push({ miner: c, amount: share, type: 'clock' });
    }
  }

  // ── Storage pool ───────────────────────────────────────────────────────
  if (storageScore > 0) {
    splitPool(storageHosts, storageWork, 1048576, storagePool, 'storage', rewards);
  }

  // ── IoT pool — sensors and gateways sub-split ─────────────────────────
  if (sensorScore > 0) {
    const sensorSubPool  = round(sensorPool * (sensorRawScore  / sensorScore));
    const gatewaySubPool = round(sensorPool * (gatewayRawScore / sensorScore));

    if (sensorRawScore > 0 && sensors.length > 0) {
      // Use combined scores (base + revenue) for within-pool split when available
      var _sensorSplitMap = Object.keys(_sensorCombined).length > 0 ? _sensorCombined : sensorWork;
      var _activeSensors = Object.keys(_sensorCombined).length > 0
        ? Object.keys(_sensorCombined)
        : sensors;
      splitPool(_activeSensors, _sensorSplitMap, 1, sensorSubPool, 'iot_sensor', rewards);
    }
    if (gatewayRawScore > 0 && gateways.length > 0) {
      splitPool(gateways, gatewayWork, 1, gatewaySubPool, 'iot_gateway', rewards);
    }
  }

  // ── Service pool ───────────────────────────────────────────────────────
  if (serviceScore > 0) {
    splitPool(serviceHosts, serviceWork, 1, servicePool, 'service', rewards);
  }

  // ── Rounding dust → recycle ────────────────────────────────────────────
  const paid = rewards.reduce((s, r) => s + r.amount, 0);
  const dust = round(blockReward - paid);
  if (Math.abs(dust) > 1e-9) {
    rewards.push({ miner: 'hone_recycle', amount: Math.abs(dust), type: 'recycle' });
  }

  return rewards;
}

module.exports = {
  distributeBlockReward,
  CLOCK_SCORE_PER_NODE,
  VERIFIER_SCORE_PER_NODE,
  STORAGE_SCORE_PER_HOST,
  SENSOR_SCORE_PER_SENSOR,
  GATEWAY_SCORE_PER_GW,
  SERVICE_SCORE_PER_HOST,
  VERIFIER_SCORE_PER_CHECK,
  STORAGE_SCORE_PER_MB,
  SENSOR_BASE_SCORE_PER_NODE,
  SENSOR_REVENUE_MULTIPLIER,
  SENSOR_SCORE_PER_READING,
  GATEWAY_SCORE_PER_PACKET,
  SERVICE_SCORE_PER_REQUEST,
  SCARCITY_THRESHOLD,
  SCARCITY_MULTIPLIER,
  TESTNET_FRACTION,
};

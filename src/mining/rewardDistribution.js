"use strict";

/**
 * BTCPC Block Emission — dynamic reward distribution
 * Shin Devlin
 *
 * Pool allocation responds to three signals:
 *
 *   1. Demand  — miner score = real verified work_value (inference demand)
 *   2. Scarcity — roles with < SCARCITY_THRESHOLD nodes get a per-node
 *                 multiplier so the chain always attracts critical infra
 *   3. Floors + caps — each active role is guaranteed a minimum slice and
 *                 capped at a maximum so no single role crowds out others
 *
 * Before the dynamic split, 0.1% is carved off the top for testnet runners —
 * people keeping the test network alive so developers have somewhere to test.
 *
 * Work scores (normalized compute units):
 *   Miner        = sum of work_value (verified parameter × job quality)
 *   Verifier     = active_verifiers × VERIFIER_SCORE_PER_NODE  [× scarcity]
 *   Clock        = active_clocks    × CLOCK_SCORE_PER_NODE     [× scarcity]
 *   Storage host = active_hosts     × STORAGE_SCORE_PER_HOST   [× scarcity]
 *   IoT sensor   = active_sensors   × SENSOR_SCORE_PER_SENSOR
 *   IoT gateway  = active_gateways  × GATEWAY_SCORE_PER_GW
 *   Service host = active_services  × SERVICE_SCORE_PER_HOST   [× scarcity]
 *
 * Participants shape:
 *   {
 *     miners:       string[]
 *     minerWork:    { [account]: number }   // raw work_value per miner
 *     verifiers:    string[]
 *     clocks:       string[]
 *     storageHosts: string[]
 *     sensors:      string[]
 *     gateways:     string[]
 *     serviceHosts: string[]
 *     testnets:     string[]               // nodes running the test network
 *   }
 */

// Base work scores per participant
const CLOCK_SCORE_PER_NODE    = 1000;
const VERIFIER_SCORE_PER_NODE = 3000;
const STORAGE_SCORE_PER_HOST  = 2000;
const SENSOR_SCORE_PER_SENSOR = 500;
const GATEWAY_SCORE_PER_GW    = 1500;
const SERVICE_SCORE_PER_HOST  = 2500;

// Scarcity: below this node count, per-node score is multiplied up
const SCARCITY_THRESHOLD  = 3;
const SCARCITY_MULTIPLIER = 2.5;

// Testnet carveout — taken before any other split
const TESTNET_FRACTION = 0.001; // 0.1%

// Floors: minimum fraction of the distributable pool guaranteed to each active role.
// Prevents critical roles from earning zero and being abandoned.
const FLOOR = {
  miner:    0.20,
  verifier: 0.03,
  clock:    0.05,
  storage:  0.03,
  iot:      0.00,
  service:  0.00,
};

// Caps: maximum fraction any role can claim, even with high participation.
const CAP = {
  miner:    0.75,
  verifier: 0.20,
  clock:    0.15,
  storage:  0.20,
  iot:      0.15,
  service:  0.15,
};

function round(n) {
  return parseFloat(Number(n).toFixed(10));
}

function scarcityScore(count, basePerNode) {
  if (count === 0) return 0;
  const perNode = count < SCARCITY_THRESHOLD
    ? basePerNode * SCARCITY_MULTIPLIER
    : basePerNode;
  return count * perNode;
}

// Constrained normalization with water-filling.
//
// Roles are divided into "fixed" (locked at floor or cap) and "free".
// Each iteration, budget remaining after fixed roles is distributed
// proportionally among free roles. Any free role that violates its bounds
// gets moved to fixed. Repeats until all free roles are in-bounds.
//
// Guaranteed to converge: each iteration moves at least one role to fixed.
function constrainedNormalize(rawScores, floors, caps, hasAny) {
  const keys = Object.keys(rawScores);
  const rawTotal = keys.reduce((s, k) => s + rawScores[k], 0);
  if (rawTotal === 0) return Object.fromEntries(keys.map(k => [k, 0]));

  const fixed = {};   // key → locked fraction (at floor or cap)
  const free = new Set(keys);

  for (let iter = 0; iter < keys.length + 2; iter++) {
    const fixedSum = Object.values(fixed).reduce((s, v) => s + v, 0);
    const remaining = 1.0 - fixedSum;
    const freeRawSum = [...free].reduce((s, k) => s + rawScores[k], 0);

    // Tentative allocation for free roles
    const tryAlloc = {};
    for (const k of free) {
      const floor = hasAny[k] ? (floors[k] || 0) : 0;
      const unconstrained = freeRawSum > 0 ? rawScores[k] / freeRawSum * remaining : 0;
      tryAlloc[k] = Math.max(floor, unconstrained);
    }

    // Check violations
    let anyViolation = false;
    for (const k of [...free]) {
      const cap = caps[k] != null ? caps[k] : 1;
      const floor = hasAny[k] ? (floors[k] || 0) : 0;
      if (tryAlloc[k] > cap + 1e-12) {
        fixed[k] = cap;
        free.delete(k);
        anyViolation = true;
      } else if (tryAlloc[k] < floor - 1e-12) {
        fixed[k] = floor;
        free.delete(k);
        anyViolation = true;
      }
    }

    if (!anyViolation) {
      // All free roles are valid — assemble result
      const result = { ...fixed };
      for (const k of free) result[k] = tryAlloc[k];
      const sum = keys.reduce((s, k) => s + result[k], 0);
      if (sum > 0) for (const k of keys) result[k] = result[k] / sum;
      return result;
    }
  }

  // Fallback: best effort from current fixed + proportional free
  const fixedSum = Object.values(fixed).reduce((s, v) => s + v, 0);
  const remaining = Math.max(0, 1.0 - fixedSum);
  const freeRawSum = [...free].reduce((s, k) => s + rawScores[k], 0);
  const result = { ...fixed };
  for (const k of free) {
    result[k] = freeRawSum > 0 ? rawScores[k] / freeRawSum * remaining : 0;
  }
  const sum = keys.reduce((s, k) => s + result[k], 0);
  if (sum > 0) for (const k of keys) result[k] = result[k] / sum;
  return result;
}

function distributeBlockReward(blockReward, participants) {
  const rewards = [];

  const miners       = participants.miners       || [];
  const minerWork    = participants.minerWork    || {};
  const verifiers    = participants.verifiers    || [];
  const clocks       = participants.clocks       || [];
  const storageHosts = participants.storageHosts || [];
  const sensors      = participants.sensors      || [];
  const gateways     = participants.gateways     || [];
  const serviceHosts = participants.serviceHosts || [];
  const testnets     = participants.testnets     || [];

  // ── 1. Testnet carveout (0.1%) ─────────────────────────────────────────
  const testnetPool    = round(blockReward * TESTNET_FRACTION);
  const distributable  = round(blockReward - testnetPool);

  if (testnets.length > 0) {
    const share = round(testnetPool / testnets.length);
    for (const t of testnets) {
      rewards.push({ miner: t, amount: share, type: 'testnet' });
    }
  } else {
    rewards.push({ miner: 'btcpc_recycle', amount: testnetPool, type: 'recycle' });
  }

  // ── 2. Raw work scores with scarcity premium ───────────────────────────
  const rawMinerScore    = miners.reduce((s, m) => s + (minerWork[m] || 0), 0);
  const rawVerifierScore = scarcityScore(verifiers.length, VERIFIER_SCORE_PER_NODE);
  const rawClockScore    = scarcityScore(clocks.length,    CLOCK_SCORE_PER_NODE);
  const rawStorageScore  = scarcityScore(storageHosts.length, STORAGE_SCORE_PER_HOST);
  const rawSensorScore   = sensors.length  * SENSOR_SCORE_PER_SENSOR
                         + gateways.length * GATEWAY_SCORE_PER_GW;
  const rawServiceScore  = scarcityScore(serviceHosts.length, SERVICE_SCORE_PER_HOST);

  const rawTotal = rawMinerScore + rawVerifierScore + rawClockScore
                 + rawStorageScore + rawSensorScore + rawServiceScore;

  // ── Nothing active — recycle the distributable pool ────────────────────
  if (rawTotal === 0) {
    rewards.push({ miner: 'btcpc_recycle', amount: distributable, type: 'recycle' });
    const paid = rewards.reduce((s, r) => s + r.amount, 0);
    const dust = round(blockReward - paid);
    if (Math.abs(dust) > 1e-9) {
      rewards.push({ miner: 'btcpc_recycle', amount: Math.abs(dust), type: 'recycle' });
    }
    return rewards;
  }

  // ── 3. Constrained normalization: floors + hard caps ──────────────────
  const hasAny = {
    miner:    miners.length > 0,
    verifier: verifiers.length > 0,
    clock:    clocks.length > 0,
    storage:  storageHosts.length > 0,
    iot:      sensors.length > 0 || gateways.length > 0,
    service:  serviceHosts.length > 0,
  };

  const rawScores = {
    miner:    rawMinerScore,
    verifier: rawVerifierScore,
    clock:    rawClockScore,
    storage:  rawStorageScore,
    iot:      rawSensorScore,
    service:  rawServiceScore,
  };

  const fractions = constrainedNormalize(rawScores, FLOOR, CAP, hasAny);

  // ── 4. Pool sizes ─────────────────────────────────────────────────────
  const minerPool    = round(distributable * fractions.miner);
  const verifierPool = round(distributable * fractions.verifier);
  const clockPool    = round(distributable * fractions.clock);
  const storagePool  = round(distributable * fractions.storage);
  const sensorPool   = round(distributable * fractions.iot);
  const servicePool  = round(distributable * fractions.service);

  // ── Miner pool — proportional to work_value ────────────────────────────
  if (rawMinerScore > 0) {
    for (const m of miners) {
      const w = minerWork[m] || 0;
      if (w > 0) {
        rewards.push({ miner: m, amount: round(minerPool * (w / rawMinerScore)), type: 'miner' });
      }
    }
  }

  // ── Verifier pool — equal split ────────────────────────────────────────
  if (rawVerifierScore > 0) {
    const share = round(verifierPool / verifiers.length);
    for (const v of verifiers) {
      rewards.push({ miner: v, amount: share, type: 'verifier' });
    }
  }

  // ── Clock pool — equal split ───────────────────────────────────────────
  if (rawClockScore > 0) {
    const share = round(clockPool / clocks.length);
    for (const c of clocks) {
      rewards.push({ miner: c, amount: share, type: 'clock' });
    }
  }

  // ── Storage pool — equal split ─────────────────────────────────────────
  if (rawStorageScore > 0) {
    const share = round(storagePool / storageHosts.length);
    for (const h of storageHosts) {
      rewards.push({ miner: h, amount: share, type: 'storage' });
    }
  }

  // ── IoT pool — proportional by sub-role score ─────────────────────────
  if (rawSensorScore > 0) {
    const sensorOnlyScore  = sensors.length  * SENSOR_SCORE_PER_SENSOR;
    const gatewayOnlyScore = gateways.length * GATEWAY_SCORE_PER_GW;
    const sensorSubPool    = round(sensorPool * (sensorOnlyScore  / rawSensorScore));
    const gatewaySubPool   = round(sensorPool * (gatewayOnlyScore / rawSensorScore));

    if (sensors.length > 0) {
      const share = round(sensorSubPool / sensors.length);
      for (const s of sensors) {
        rewards.push({ miner: s, amount: share, type: 'iot_sensor' });
      }
    }
    if (gateways.length > 0) {
      const share = round(gatewaySubPool / gateways.length);
      for (const g of gateways) {
        rewards.push({ miner: g, amount: share, type: 'iot_gateway' });
      }
    }
  }

  // ── Service pool — equal split ─────────────────────────────────────────
  if (rawServiceScore > 0) {
    const share = round(servicePool / serviceHosts.length);
    for (const h of serviceHosts) {
      rewards.push({ miner: h, amount: share, type: 'service' });
    }
  }

  // ── Rounding dust → recycle ────────────────────────────────────────────
  const paid = rewards.reduce((s, r) => s + r.amount, 0);
  const dust = round(blockReward - paid);
  if (Math.abs(dust) > 1e-9) {
    rewards.push({ miner: 'btcpc_recycle', amount: Math.abs(dust), type: 'recycle' });
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
  SCARCITY_THRESHOLD,
  SCARCITY_MULTIPLIER,
  TESTNET_FRACTION,
  FLOOR,
  CAP,
};

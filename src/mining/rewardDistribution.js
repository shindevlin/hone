"use strict";

/**
 * BTCPC Block Emission — fully dynamic reward distribution
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
 * Work scores (demand signal per role):
 *   Miner        = sum of verified work_value (inference compute delivered)
 *   Verifier     = node count × VERIFIER_SCORE [× scarcity if < threshold]
 *   Clock        = node count × CLOCK_SCORE    [× scarcity if < threshold]
 *   Storage host = bytes_stored × retrievals   (→ STORAGE_SCORE_PER_HOST proxy until live)
 *   IoT sensor   = readings_delivered × data_value (→ SENSOR_SCORE proxy until live)
 *   IoT gateway  = packets_relayed    (→ GATEWAY_SCORE proxy until live)
 *   Service host = requests_served    (→ SERVICE_SCORE proxy until live)
 *
 * Scarcity: roles with fewer than SCARCITY_THRESHOLD nodes get a multiplier
 * so critical infrastructure is always worth running, even when the network
 * is small. As participation grows the premium fades naturally.
 *
 * Participants shape:
 *   {
 *     miners:       string[]
 *     minerWork:    { [account]: number }   // verified work_value per miner
 *     verifiers:    string[]
 *     clocks:       string[]
 *     storageHosts: string[]
 *     sensors:      string[]
 *     gateways:     string[]
 *     serviceHosts: string[]               // nodes hosting BTCPC services
 *     testnets:     string[]               // nodes running the test network
 *   }
 */

// Per-node base scores — proxy until live demand data is available
const CLOCK_SCORE_PER_NODE    = 1000;
const VERIFIER_SCORE_PER_NODE = 3000;
const STORAGE_SCORE_PER_HOST  = 2000;
const SENSOR_SCORE_PER_SENSOR = 500;
const GATEWAY_SCORE_PER_GW    = 1500;
const SERVICE_SCORE_PER_HOST  = 2500;

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
  const testnetPool   = round(blockReward * TESTNET_FRACTION);
  const distributable = round(blockReward - testnetPool);

  if (testnets.length > 0) {
    const share = round(testnetPool / testnets.length);
    for (const t of testnets) {
      rewards.push({ miner: t, amount: share, type: 'testnet' });
    }
  } else {
    rewards.push({ miner: 'btcpc_recycle', amount: testnetPool, type: 'recycle' });
  }

  // ── 2. Work scores — fully demand-driven ──────────────────────────────
  const minerScore   = miners.reduce((s, m) => s + (minerWork[m] || 0), 0);
  const verifierScore = scarcityScore(verifiers.length, VERIFIER_SCORE_PER_NODE);
  const clockScore    = scarcityScore(clocks.length,    CLOCK_SCORE_PER_NODE);
  const storageScore  = scarcityScore(storageHosts.length, STORAGE_SCORE_PER_HOST);
  const sensorScore   = sensors.length  * SENSOR_SCORE_PER_SENSOR
                      + gateways.length * GATEWAY_SCORE_PER_GW;
  const serviceScore  = scarcityScore(serviceHosts.length, SERVICE_SCORE_PER_HOST);

  const totalScore = minerScore + verifierScore + clockScore
                   + storageScore + sensorScore + serviceScore;

  // ── Nothing active — recycle the distributable pool ───────────────────
  if (totalScore === 0) {
    rewards.push({ miner: 'btcpc_recycle', amount: distributable, type: 'recycle' });
    const paid = rewards.reduce((s, r) => s + r.amount, 0);
    const dust = round(blockReward - paid);
    if (Math.abs(dust) > 1e-9) {
      rewards.push({ miner: 'btcpc_recycle', amount: Math.abs(dust), type: 'recycle' });
    }
    return rewards;
  }

  // ── 3. Pool sizes — purely proportional ───────────────────────────────
  const minerPool   = round(distributable * (minerScore   / totalScore));
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

  // ── Verifier pool — equal split ────────────────────────────────────────
  if (verifierScore > 0) {
    const share = round(verifierPool / verifiers.length);
    for (const v of verifiers) {
      rewards.push({ miner: v, amount: share, type: 'verifier' });
    }
  }

  // ── Clock pool — equal split ───────────────────────────────────────────
  if (clockScore > 0) {
    const share = round(clockPool / clocks.length);
    for (const c of clocks) {
      rewards.push({ miner: c, amount: share, type: 'clock' });
    }
  }

  // ── Storage pool — equal split ─────────────────────────────────────────
  if (storageScore > 0) {
    const share = round(storagePool / storageHosts.length);
    for (const h of storageHosts) {
      rewards.push({ miner: h, amount: share, type: 'storage' });
    }
  }

  // ── IoT pool — proportional by sub-role score ─────────────────────────
  if (sensorScore > 0) {
    const sensorOnlyScore  = sensors.length  * SENSOR_SCORE_PER_SENSOR;
    const gatewayOnlyScore = gateways.length * GATEWAY_SCORE_PER_GW;
    const sensorSubPool    = round(sensorPool * (sensorOnlyScore  / sensorScore));
    const gatewaySubPool   = round(sensorPool * (gatewayOnlyScore / sensorScore));

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
  if (serviceScore > 0) {
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
};

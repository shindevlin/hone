"use strict";

/**
 * BTCPC Block Emission — demand-driven reward distribution
 * Shin Devlin
 *
 * Each role earns proportional to its normalized work score, not a static
 * percentage. The pool split self-adjusts to actual network activity each
 * epoch. If AI inference is 90% of demand, miners get 90% of the reward.
 *
 * Work scores (normalized compute units):
 *   Miner        = sum of work_value (verified parameter × job quality)
 *   Verifier     = active_verifiers × VERIFIER_SCORE_PER_NODE
 *   Clock        = active_clocks    × CLOCK_SCORE_PER_NODE
 *   Storage host = active_hosts     × STORAGE_SCORE_PER_HOST
 *   IoT sensor   = active_sensors   × SENSOR_SCORE_PER_SENSOR
 *   IoT gateway  = active_gateways  × GATEWAY_SCORE_PER_GW
 *   Service host = active_services  × SERVICE_SCORE_PER_HOST
 *
 * Calibrated so at typical participation levels the split approximates the
 * original whitepaper ratios (~56% miners, ~10% each verifier/storage/IoT,
 * ~5% clocks), but shifts naturally with actual demand.
 *
 * Empty pools: nothing recycles from non-participation — the weight just
 * goes to zero and other active roles proportionally inherit it.
 * True recycle only when totalScore = 0 (no activity at all).
 *
 * Participants shape:
 *   {
 *     miners:       string[]
 *     minerWork:    { [account]: number }   // raw work_value per miner
 *     verifiers:    string[]
 *     clocks:       string[]
 *     storageHosts: string[]
 *     sensors:      string[]                // optional, IoT sensor owners
 *     gateways:     string[]                // optional, IoT gateway owners
 *     serviceHosts: string[]                // optional, future service hosting
 *   }
 */

// Per-node work scores — each equals the compute units that role contributes
const CLOCK_SCORE_PER_NODE    = 1000;
const VERIFIER_SCORE_PER_NODE = 3000;
const STORAGE_SCORE_PER_HOST  = 2000;
const SENSOR_SCORE_PER_SENSOR = 500;
const GATEWAY_SCORE_PER_GW    = 1500;
const SERVICE_SCORE_PER_HOST  = 2500;

function round(n) {
  return parseFloat(Number(n).toFixed(10));
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

  // ── Compute work scores per role ──────────────────────────────────────
  const minerScore    = miners.reduce((s, m) => s + (minerWork[m] || 0), 0);
  const verifierScore = verifiers.length * VERIFIER_SCORE_PER_NODE;
  const clockScore    = clocks.length    * CLOCK_SCORE_PER_NODE;
  const storageScore  = storageHosts.length * STORAGE_SCORE_PER_HOST;
  const sensorScore   = sensors.length   * SENSOR_SCORE_PER_SENSOR
                      + gateways.length  * GATEWAY_SCORE_PER_GW;
  const serviceScore  = serviceHosts.length * SERVICE_SCORE_PER_HOST;

  const totalScore = minerScore + verifierScore + clockScore + storageScore
                   + sensorScore + serviceScore;

  // ── Nothing happening — full recycle ─────────────────────────────────
  if (totalScore === 0) {
    rewards.push({ miner: 'btcpc_recycle', amount: round(blockReward), type: 'recycle' });
    return rewards;
  }

  // ── Dynamic pool sizes ────────────────────────────────────────────────
  const minerPool    = round(blockReward * (minerScore    / totalScore));
  const verifierPool = round(blockReward * (verifierScore / totalScore));
  const clockPool    = round(blockReward * (clockScore    / totalScore));
  const storagePool  = round(blockReward * (storageScore  / totalScore));
  const sensorPool   = round(blockReward * (sensorScore   / totalScore));
  const servicePool  = round(blockReward * (serviceScore  / totalScore));

  // ── Miner pool — proportional to work_value ───────────────────────────
  if (minerScore > 0) {
    for (const m of miners) {
      const w = minerWork[m] || 0;
      if (w > 0) {
        rewards.push({ miner: m, amount: round(minerPool * (w / minerScore)), type: 'miner' });
      }
    }
  }

  // ── Verifier pool — equal split ───────────────────────────────────────
  if (verifierScore > 0) {
    const share = round(verifierPool / verifiers.length);
    for (const v of verifiers) {
      rewards.push({ miner: v, amount: share, type: 'verifier' });
    }
  }

  // ── Clock pool — equal split ──────────────────────────────────────────
  if (clockScore > 0) {
    const share = round(clockPool / clocks.length);
    for (const c of clocks) {
      rewards.push({ miner: c, amount: share, type: 'clock' });
    }
  }

  // ── Storage pool — equal split ────────────────────────────────────────
  if (storageScore > 0) {
    const share = round(storagePool / storageHosts.length);
    for (const h of storageHosts) {
      rewards.push({ miner: h, amount: share, type: 'storage' });
    }
  }

  // ── IoT pool — 60% sensors (proportional) + 40% gateways (equal) ─────
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

  // ── Service pool — equal split (future) ───────────────────────────────
  if (serviceScore > 0) {
    const share = round(servicePool / serviceHosts.length);
    for (const h of serviceHosts) {
      rewards.push({ miner: h, amount: share, type: 'service' });
    }
  }

  // ── Rounding dust → recycle ───────────────────────────────────────────
  const paid = rewards.reduce((s, r) => s + r.amount, 0);
  const dust = round(blockReward - paid);
  if (Math.abs(dust) > 1e-9) {
    rewards.push({ miner: 'btcpc_recycle', amount: Math.abs(dust), type: 'recycle' });
  }

  return rewards;
}

// Expose scores so callers can display or reason about the dynamic split
module.exports = {
  distributeBlockReward,
  CLOCK_SCORE_PER_NODE,
  VERIFIER_SCORE_PER_NODE,
  STORAGE_SCORE_PER_HOST,
  SENSOR_SCORE_PER_SENSOR,
  GATEWAY_SCORE_PER_GW,
  SERVICE_SCORE_PER_HOST,
};

"use strict";

/**
 * BTCPC Nano Rewards — v2.15-alpha
 * Shin Devlin
 *
 * Reward calculation for the BTCPC-nano IoT pool.
 *
 * All BTCPC amounts are percentages of a pool — never fixed quantities.
 * See feedback_conditional_payouts.md: "no fixed BTCPC promises in the
 * protocol layer". The caller (miner.js, v2.15-beta) decides what
 * fraction of the block reward constitutes the IoT pool; this module
 * only computes how that pool is split among participants.
 *
 * No burn ever: any fees flow to btcpc_recycle, never to a burn address.
 * See feedback_no_burn_all_recycle.md.
 *
 * No slashing for offline sensors or gateways — paid for delivery only.
 * See feedback_storage_no_slash.md.
 *
 * Split model:
 *   60% of IoT pool → active sensors (weighted by readings × uptime_pct)
 *   40% of IoT pool → active gateways (weighted by packets_relayed × uptime_pct)
 *
 * If no active sensors, their 60% rolls into the gateway pool (and vice versa).
 * This avoids wasting emissions while the network is sparse.
 */

var SENSOR_POOL_FRACTION = 0.60;
var GATEWAY_POOL_FRACTION = 0.40;

/**
 * Compute IoT rewards for an epoch.
 *
 * @param {number} epochNumber — the epoch being rewarded
 * @param {number} blockReward — the total IoT pool for this epoch (caller-defined fraction of block reward)
 * @param {Array} activeGateways — array of { gateway_id, owner, packets_relayed, uptime_pct }
 * @param {Array} activeSensors — array of { sensor_id, owner, readings, uptime_pct }
 * @returns {Array} — [{ account, amount, type: 'sensor'|'gateway', device_id }]
 */
function computeIoTRewards(epochNumber, blockReward, activeGateways, activeSensors) {
  if (!Number.isFinite(blockReward) || blockReward <= 0) {
    return [];
  }

  var sensors = Array.isArray(activeSensors) ? activeSensors : [];
  var gateways = Array.isArray(activeGateways) ? activeGateways : [];

  var sensorPool = blockReward * SENSOR_POOL_FRACTION;
  var gatewayPool = blockReward * GATEWAY_POOL_FRACTION;

  // Compute sensor weights: readings * uptime_pct
  var sensorWeights = sensors.map(function(s) {
    var readings = Number(s.readings) || 0;
    var uptime = Number(s.uptime_pct) || 0;
    return { device: s, weight: readings * uptime };
  });
  var totalSensorWeight = sensorWeights.reduce(function(sum, sw) { return sum + sw.weight; }, 0);

  // Compute gateway weights: packets_relayed * uptime_pct
  var gatewayWeights = gateways.map(function(g) {
    var packets = Number(g.packets_relayed) || 0;
    var uptime = Number(g.uptime_pct) || 0;
    return { device: g, weight: packets * uptime };
  });
  var totalGatewayWeight = gatewayWeights.reduce(function(sum, gw) { return sum + gw.weight; }, 0);

  // Roll over empty pools
  var effectiveSensorPool = sensorPool;
  var effectiveGatewayPool = gatewayPool;

  if (totalSensorWeight === 0 && totalGatewayWeight > 0) {
    // No active sensors: all to gateways
    effectiveGatewayPool = blockReward;
    effectiveSensorPool = 0;
  } else if (totalGatewayWeight === 0 && totalSensorWeight > 0) {
    // No active gateways: all to sensors
    effectiveSensorPool = blockReward;
    effectiveGatewayPool = 0;
  } else if (totalSensorWeight === 0 && totalGatewayWeight === 0) {
    // Nothing active — no rewards issued
    return [];
  }

  var rewards = [];

  // Sensor payouts
  if (effectiveSensorPool > 0 && totalSensorWeight > 0) {
    for (var i = 0; i < sensorWeights.length; i++) {
      var sw = sensorWeights[i];
      if (sw.weight <= 0) continue;
      var share = (sw.weight / totalSensorWeight) * effectiveSensorPool;
      rewards.push({
        account: sw.device.owner,
        amount: share,
        type: "sensor",
        device_id: sw.device.sensor_id,
        epoch: epochNumber,
        weight: sw.weight,
      });
    }
  }

  // Gateway payouts
  if (effectiveGatewayPool > 0 && totalGatewayWeight > 0) {
    for (var j = 0; j < gatewayWeights.length; j++) {
      var gw = gatewayWeights[j];
      if (gw.weight <= 0) continue;
      var gshare = (gw.weight / totalGatewayWeight) * effectiveGatewayPool;
      rewards.push({
        account: gw.device.owner,
        amount: gshare,
        type: "gateway",
        device_id: gw.device.gateway_id,
        epoch: epochNumber,
        weight: gw.weight,
      });
    }
  }

  return rewards;
}

/**
 * Rough per-sensor earnings estimate for UX display.
 *
 * Returns the expected share of the sensor pool per epoch given:
 *   - how many readings this sensor submits per epoch
 *   - this sensor's uptime percentage
 *   - the total number of sensors in the pool
 *
 * The result is expressed as a fraction of the total IoT pool so the
 * caller can multiply by whatever block reward they want to display.
 *
 * Never returns a fixed BTCPC amount — always a fraction.
 *
 * @param {number} readingsPerEpoch — readings this sensor submits per epoch
 * @param {number} uptimePct — sensor uptime as a decimal (0.0 – 1.0)
 * @param {number} totalSensors — estimated total active sensors in the pool
 * @returns {number} — estimated fraction of total IoT pool this sensor earns per epoch
 */
function estimateEarnings(readingsPerEpoch, uptimePct, totalSensors) {
  var readings = Math.max(0, Number(readingsPerEpoch) || 0);
  var uptime = Math.min(1, Math.max(0, Number(uptimePct) || 0));
  var n = Math.max(1, Number(totalSensors) || 1);

  if (readings === 0 || uptime === 0) return 0;

  // Assume all sensors have the same readings+uptime (worst-case fair share)
  // This sensor's weight / total weight = 1/n (since all equal)
  // Fraction of IoT pool = (1/n) * SENSOR_POOL_FRACTION
  var weightedShare = (readings * uptime) / (n * readings * uptime);
  return weightedShare * SENSOR_POOL_FRACTION;
}

module.exports = {
  computeIoTRewards: computeIoTRewards,
  estimateEarnings: estimateEarnings,
  SENSOR_POOL_FRACTION: SENSOR_POOL_FRACTION,
  GATEWAY_POOL_FRACTION: GATEWAY_POOL_FRACTION,
};

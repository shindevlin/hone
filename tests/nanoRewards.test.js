"use strict";

/**
 * Nano Rewards tests — v2.15-alpha
 *
 * Covers 60/40 split, proportional weighting, edge cases (zero sensors,
 * zero gateways, single participant), and the estimateEarnings helper.
 */

const nr = require('../src/services/nanoRewards');

function makeSensors(count, readingsPerEpoch, uptimePct) {
  var sensors = [];
  for (var i = 0; i < count; i++) {
    sensors.push({
      sensor_id: 'owner/sensor-' + i,
      owner: 'owner',
      readings: readingsPerEpoch,
      uptime_pct: uptimePct,
    });
  }
  return sensors;
}

function makeGateways(count, packets, uptimePct) {
  var gateways = [];
  for (var i = 0; i < count; i++) {
    gateways.push({
      gateway_id: 'owner/gateway-' + i,
      owner: 'owner',
      packets_relayed: packets,
      uptime_pct: uptimePct,
    });
  }
  return gateways;
}

describe('nanoRewards — 60/40 sensor/gateway split', () => {
  it('SENSOR_POOL_FRACTION is 0.60', () => {
    expect(nr.SENSOR_POOL_FRACTION).toBe(0.60);
  });

  it('GATEWAY_POOL_FRACTION is 0.40', () => {
    expect(nr.GATEWAY_POOL_FRACTION).toBe(0.40);
  });

  it('sensor rewards sum to 60% when both pools are active', () => {
    const blockReward = 1000;
    const sensors = makeSensors(2, 10, 1.0);
    const gateways = makeGateways(2, 100, 1.0);
    const rewards = nr.computeIoTRewards(1, blockReward, gateways, sensors);
    const sensorTotal = rewards
      .filter(r => r.type === 'sensor')
      .reduce((s, r) => s + r.amount, 0);
    const gatewayTotal = rewards
      .filter(r => r.type === 'gateway')
      .reduce((s, r) => s + r.amount, 0);
    expect(sensorTotal).toBeCloseTo(600, 6);
    expect(gatewayTotal).toBeCloseTo(400, 6);
  });

  it('total rewards sum to full blockReward', () => {
    const blockReward = 243.06;
    const sensors = makeSensors(3, 5, 0.9);
    const gateways = makeGateways(1, 200, 0.8);
    const rewards = nr.computeIoTRewards(5, blockReward, gateways, sensors);
    const total = rewards.reduce((s, r) => s + r.amount, 0);
    expect(total).toBeCloseTo(blockReward, 6);
  });
});

describe('nanoRewards — proportional distribution', () => {
  it('rewards are proportional to readings * uptime_pct', () => {
    const blockReward = 1000;
    const sensors = [
      { sensor_id: 'a/s1', owner: 'a', readings: 10, uptime_pct: 1.0 }, // weight 10
      { sensor_id: 'b/s2', owner: 'b', readings: 20, uptime_pct: 1.0 }, // weight 20 (2x)
    ];
    const gateways = makeGateways(1, 100, 1.0);
    const rewards = nr.computeIoTRewards(1, blockReward, gateways, sensors);
    const r1 = rewards.find(r => r.device_id === 'a/s1');
    const r2 = rewards.find(r => r.device_id === 'b/s2');
    expect(r2.amount).toBeCloseTo(r1.amount * 2, 6);
  });

  it('rewards are proportional to packets_relayed * uptime_pct for gateways', () => {
    const blockReward = 1000;
    const sensors = makeSensors(1, 10, 1.0);
    const gateways = [
      { gateway_id: 'a/g1', owner: 'a', packets_relayed: 50, uptime_pct: 1.0 },
      { gateway_id: 'b/g2', owner: 'b', packets_relayed: 150, uptime_pct: 1.0 }, // 3x
    ];
    const rewards = nr.computeIoTRewards(1, blockReward, gateways, sensors);
    const g1 = rewards.find(r => r.device_id === 'a/g1');
    const g2 = rewards.find(r => r.device_id === 'b/g2');
    expect(g2.amount).toBeCloseTo(g1.amount * 3, 6);
  });

  it('single sensor gets full sensor pool', () => {
    const blockReward = 1000;
    const sensors = [{ sensor_id: 'a/s1', owner: 'a', readings: 10, uptime_pct: 1.0 }];
    const gateways = makeGateways(1, 100, 1.0);
    const rewards = nr.computeIoTRewards(1, blockReward, gateways, sensors);
    const sReward = rewards.find(r => r.type === 'sensor');
    expect(sReward.amount).toBeCloseTo(600, 6);
  });

  it('single gateway gets full gateway pool', () => {
    const blockReward = 1000;
    const sensors = makeSensors(1, 10, 1.0);
    const gateways = [{ gateway_id: 'a/g1', owner: 'a', packets_relayed: 100, uptime_pct: 1.0 }];
    const rewards = nr.computeIoTRewards(1, blockReward, gateways, sensors);
    const gReward = rewards.find(r => r.type === 'gateway');
    expect(gReward.amount).toBeCloseTo(400, 6);
  });
});

describe('nanoRewards — edge cases', () => {
  it('zero sensors → all blockReward goes to gateways', () => {
    const blockReward = 1000;
    const gateways = makeGateways(2, 100, 1.0);
    const rewards = nr.computeIoTRewards(1, blockReward, gateways, []);
    const sensorTotal = rewards.filter(r => r.type === 'sensor').reduce((s, r) => s + r.amount, 0);
    const gatewayTotal = rewards.filter(r => r.type === 'gateway').reduce((s, r) => s + r.amount, 0);
    expect(sensorTotal).toBe(0);
    expect(gatewayTotal).toBeCloseTo(blockReward, 6);
  });

  it('zero gateways → all blockReward goes to sensors', () => {
    const blockReward = 1000;
    const sensors = makeSensors(2, 10, 1.0);
    const rewards = nr.computeIoTRewards(1, blockReward, [], sensors);
    const sensorTotal = rewards.filter(r => r.type === 'sensor').reduce((s, r) => s + r.amount, 0);
    const gatewayTotal = rewards.filter(r => r.type === 'gateway').reduce((s, r) => s + r.amount, 0);
    expect(sensorTotal).toBeCloseTo(blockReward, 6);
    expect(gatewayTotal).toBe(0);
  });

  it('zero sensors and zero gateways → empty rewards array', () => {
    const rewards = nr.computeIoTRewards(1, 1000, [], []);
    expect(rewards).toEqual([]);
  });

  it('zero or negative blockReward returns empty array', () => {
    const sensors = makeSensors(2, 10, 1.0);
    const gateways = makeGateways(1, 50, 1.0);
    expect(nr.computeIoTRewards(1, 0, gateways, sensors)).toEqual([]);
    expect(nr.computeIoTRewards(1, -100, gateways, sensors)).toEqual([]);
  });

  it('sensors with zero uptime_pct get no reward', () => {
    const blockReward = 1000;
    const sensors = [
      { sensor_id: 'a/s1', owner: 'a', readings: 10, uptime_pct: 0 },
      { sensor_id: 'b/s2', owner: 'b', readings: 10, uptime_pct: 1.0 },
    ];
    const gateways = makeGateways(1, 100, 1.0);
    const rewards = nr.computeIoTRewards(1, blockReward, gateways, sensors);
    const r1 = rewards.find(r => r.device_id === 'a/s1');
    const r2 = rewards.find(r => r.device_id === 'b/s2');
    expect(r1).toBeUndefined(); // zero weight excluded
    expect(r2.amount).toBeCloseTo(600, 6);
  });

  it('reward entries include epoch, account, device_id, type', () => {
    const sensors = [{ sensor_id: 'a/s1', owner: 'a', readings: 5, uptime_pct: 1.0 }];
    const gateways = [{ gateway_id: 'a/g1', owner: 'a', packets_relayed: 10, uptime_pct: 1.0 }];
    const rewards = nr.computeIoTRewards(42, 500, gateways, sensors);
    for (const r of rewards) {
      expect(r.account).toBe('a');
      expect(r.epoch).toBe(42);
      expect(['sensor', 'gateway']).toContain(r.type);
      expect(r.device_id).toBeTruthy();
    }
  });
});

describe('nanoRewards — estimateEarnings', () => {
  it('returns a fraction between 0 and 1', () => {
    const frac = nr.estimateEarnings(10, 0.9, 50);
    expect(frac).toBeGreaterThan(0);
    expect(frac).toBeLessThanOrEqual(1);
  });

  it('with 1 sensor and full uptime, returns SENSOR_POOL_FRACTION', () => {
    const frac = nr.estimateEarnings(10, 1.0, 1);
    expect(frac).toBeCloseTo(nr.SENSOR_POOL_FRACTION, 6);
  });

  it('with N identical sensors, each earns SENSOR_POOL_FRACTION / N', () => {
    const N = 10;
    const frac = nr.estimateEarnings(10, 1.0, N);
    expect(frac).toBeCloseTo(nr.SENSOR_POOL_FRACTION / N, 6);
  });

  it('returns 0 if uptime is 0', () => {
    expect(nr.estimateEarnings(10, 0, 50)).toBe(0);
  });

  it('returns 0 if readings are 0', () => {
    expect(nr.estimateEarnings(0, 1.0, 50)).toBe(0);
  });

  it('returns reasonable number for typical IoT node setup', () => {
    // 1 reading per epoch, 80% uptime, 500 total sensors
    const frac = nr.estimateEarnings(1, 0.8, 500);
    expect(frac).toBeGreaterThan(0);
    expect(frac).toBeLessThan(0.01); // should be small in a large pool
  });
});

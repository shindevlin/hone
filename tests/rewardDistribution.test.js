"use strict";

/**
 * Tests for demand-driven reward distribution.
 * The pool split is proportional to work scores, not static percentages.
 * Shin Devlin
 */

const {
  distributeBlockReward,
  CLOCK_SCORE_PER_NODE,
  VERIFIER_SCORE_PER_NODE,
  STORAGE_SCORE_PER_HOST,
  SENSOR_SCORE_PER_SENSOR,
  GATEWAY_SCORE_PER_GW,
} = require('../src/mining/rewardDistribution');

const BLOCK_REWARD = 243.06;
const EPSILON = 1e-6;

function totalPaid(rewards) {
  return rewards.reduce((s, r) => s + r.amount, 0);
}
function byType(rewards, type) {
  return rewards.filter(r => r.type === type);
}

// ─────────────────────────────────────────────────────────────────
// Invariant: total paid always equals block reward
// ─────────────────────────────────────────────────────────────────
describe('total conservation', () => {
  const cases = [
    { label: 'all roles',
      p: { miners: ['m1','m2'], minerWork: { m1: 100, m2: 50 },
           verifiers: ['v1'], clocks: ['c1'],
           storageHosts: ['s1'], sensors: ['sen1'], gateways: ['gw1'], serviceHosts: [] } },
    { label: 'miners only',
      p: { miners: ['m1'], minerWork: { m1: 1000 }, verifiers: [], clocks: [],
           storageHosts: [], sensors: [], gateways: [], serviceHosts: [] } },
    { label: 'clocks only',
      p: { miners: [], minerWork: {}, verifiers: [], clocks: ['c1', 'c2'],
           storageHosts: [], sensors: [], gateways: [], serviceHosts: [] } },
    { label: 'storage only',
      p: { miners: [], minerWork: {}, verifiers: [], clocks: [],
           storageHosts: ['h1'], sensors: [], gateways: [], serviceHosts: [] } },
    { label: 'nothing',
      p: { miners: [], minerWork: {}, verifiers: [], clocks: [],
           storageHosts: [], sensors: [], gateways: [], serviceHosts: [] } },
  ];

  cases.forEach(({ label, p }) => {
    test(`total paid = block reward (${label})`, () => {
      const rewards = distributeBlockReward(BLOCK_REWARD, p);
      expect(Math.abs(totalPaid(rewards) - BLOCK_REWARD)).toBeLessThan(EPSILON);
    });
  });
});

// ─────────────────────────────────────────────────────────────────
// Nothing active → full recycle
// ─────────────────────────────────────────────────────────────────
describe('nothing active', () => {
  const rewards = distributeBlockReward(BLOCK_REWARD, {
    miners: [], minerWork: {}, verifiers: [], clocks: [],
    storageHosts: [], sensors: [], gateways: [], serviceHosts: [],
  });

  test('single recycle entry', () => {
    expect(rewards).toHaveLength(1);
    expect(rewards[0].type).toBe('recycle');
    expect(rewards[0].miner).toBe('btcpc_recycle');
  });

  test('recycle amount equals block reward', () => {
    expect(Math.abs(rewards[0].amount - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Dynamic split: each role gets proportional to its score
// ─────────────────────────────────────────────────────────────────
describe('dynamic split proportionality', () => {
  const MINER_WORK = 10000;
  const p = {
    miners:       ['miner1'],
    minerWork:    { miner1: MINER_WORK },
    verifiers:    ['v1'],
    clocks:       ['c1'],
    storageHosts: ['s1'],
    sensors:      [],
    gateways:     [],
    serviceHosts: [],
  };
  const totalScore = MINER_WORK
    + VERIFIER_SCORE_PER_NODE
    + CLOCK_SCORE_PER_NODE
    + STORAGE_SCORE_PER_HOST;

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, p); });

  test('miner share ≈ minerWork / totalScore', () => {
    const minerPaid = byType(rewards, 'miner').reduce((s, r) => s + r.amount, 0);
    const expected  = BLOCK_REWARD * (MINER_WORK / totalScore);
    expect(Math.abs(minerPaid - expected)).toBeLessThan(EPSILON);
  });

  test('verifier share ≈ VERIFIER_SCORE / totalScore', () => {
    const vPaid    = byType(rewards, 'verifier').reduce((s, r) => s + r.amount, 0);
    const expected = BLOCK_REWARD * (VERIFIER_SCORE_PER_NODE / totalScore);
    expect(Math.abs(vPaid - expected)).toBeLessThan(EPSILON);
  });

  test('clock share ≈ CLOCK_SCORE / totalScore', () => {
    const cPaid    = byType(rewards, 'clock').reduce((s, r) => s + r.amount, 0);
    const expected = BLOCK_REWARD * (CLOCK_SCORE_PER_NODE / totalScore);
    expect(Math.abs(cPaid - expected)).toBeLessThan(EPSILON);
  });

  test('storage share ≈ STORAGE_SCORE / totalScore', () => {
    const sPaid    = byType(rewards, 'storage').reduce((s, r) => s + r.amount, 0);
    const expected = BLOCK_REWARD * (STORAGE_SCORE_PER_HOST / totalScore);
    expect(Math.abs(sPaid - expected)).toBeLessThan(EPSILON);
  });

  test('no recycle when all roles active', () => {
    expect(byType(rewards, 'recycle')).toHaveLength(0);
  });
});

// ─────────────────────────────────────────────────────────────────
// Absent roles: their weight simply goes to zero, not recycled
// ─────────────────────────────────────────────────────────────────
describe('absent role weight flows to active roles', () => {
  // Only a miner and a clock — storage and verifiers absent
  const MINER_WORK = 5000;
  const p = {
    miners:       ['m1'],
    minerWork:    { m1: MINER_WORK },
    verifiers:    [],
    clocks:       ['c1'],
    storageHosts: [],
    sensors:      [],
    gateways:     [],
    serviceHosts: [],
  };
  const totalScore = MINER_WORK + CLOCK_SCORE_PER_NODE;

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, p); });

  test('no recycle entry (absent roles do not create recycle)', () => {
    expect(byType(rewards, 'recycle')).toHaveLength(0);
  });

  test('miner + clock together receive full block reward', () => {
    const claimed = totalPaid(rewards);
    expect(Math.abs(claimed - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });

  test('miner share = minerWork / (minerWork + clockScore)', () => {
    const minerPaid = byType(rewards, 'miner')[0].amount;
    const expected  = BLOCK_REWARD * (MINER_WORK / totalScore);
    expect(Math.abs(minerPaid - expected)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Proportional miner split
// ─────────────────────────────────────────────────────────────────
describe('proportional miner split', () => {
  // Two miners, 2:1 work ratio — no clocks, verifiers, or storage
  const p = {
    miners:       ['miner-a', 'miner-b'],
    minerWork:    { 'miner-a': 200, 'miner-b': 100 },
    verifiers:    [],
    clocks:       [],
    storageHosts: [],
    sensors:      [],
    gateways:     [],
    serviceHosts: [],
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, p); });

  test('miner-a gets 2× miner-b', () => {
    const a = byType(rewards, 'miner').find(r => r.miner === 'miner-a').amount;
    const b = byType(rewards, 'miner').find(r => r.miner === 'miner-b').amount;
    expect(Math.abs(a / b - 2)).toBeLessThan(1e-8);
  });

  test('total paid equals block reward', () => {
    expect(Math.abs(totalPaid(rewards) - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// IoT pool: sensors and gateways split by their scores
// ─────────────────────────────────────────────────────────────────
describe('IoT pool split', () => {
  const p = {
    miners: [], minerWork: {}, verifiers: [], clocks: [], storageHosts: [],
    sensors:  ['s1', 's2'],
    gateways: ['gw1'],
    serviceHosts: [],
  };
  const sensorScore  = 2 * SENSOR_SCORE_PER_SENSOR;
  const gatewayScore = 1 * GATEWAY_SCORE_PER_GW;
  const totalScore   = sensorScore + gatewayScore;

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, p); });

  test('sensor entries present', () => {
    expect(byType(rewards, 'iot_sensor')).toHaveLength(2);
  });

  test('gateway entry present', () => {
    expect(byType(rewards, 'iot_gateway')).toHaveLength(1);
  });

  test('total IoT ≈ full block reward (only IoT active)', () => {
    const iotPaid = byType(rewards, 'iot_sensor').reduce((s, r) => s + r.amount, 0)
                  + byType(rewards, 'iot_gateway').reduce((s, r) => s + r.amount, 0);
    expect(Math.abs(iotPaid - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });

  test('gateway gets proportionally more per node than sensor', () => {
    const sensorPerNode  = byType(rewards, 'iot_sensor')[0].amount;
    const gatewayPerNode = byType(rewards, 'iot_gateway')[0].amount;
    // gateway score per node > sensor score per node, so gatewayPerNode > sensorPerNode
    expect(gatewayPerNode).toBeGreaterThan(sensorPerNode);
  });

  test('total paid equals block reward', () => {
    expect(Math.abs(totalPaid(rewards) - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Calibration: typical participation → reasonable split
// ─────────────────────────────────────────────────────────────────
describe('calibration at typical participation', () => {
  // 5 miners × 10000, 10 clocks, 3 verifiers, 5 storage, 10 sensors, 3 gateways
  const p = {
    miners:       ['m1','m2','m3','m4','m5'],
    minerWork:    { m1:10000,m2:10000,m3:10000,m4:10000,m5:10000 },
    verifiers:    ['v1','v2','v3'],
    clocks:       ['c1','c2','c3','c4','c5','c6','c7','c8','c9','c10'],
    storageHosts: ['s1','s2','s3','s4','s5'],
    sensors:      ['sen1','sen2','sen3','sen4','sen5','sen6','sen7','sen8','sen9','sen10'],
    gateways:     ['gw1','gw2','gw3'],
    serviceHosts: [],
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, p); });

  test('miners get > 40% of reward (dominant role)', () => {
    const minerPct = byType(rewards, 'miner').reduce((s,r) => s+r.amount, 0) / BLOCK_REWARD;
    expect(minerPct).toBeGreaterThan(0.40);
  });

  test('each role gets at least 5% (no role starved)', () => {
    const roles = ['miner','verifier','clock','storage'];
    for (const role of roles) {
      const pct = byType(rewards, role).reduce((s,r) => s+r.amount, 0) / BLOCK_REWARD;
      expect(pct).toBeGreaterThan(0.05);
    }
  });

  test('total paid equals block reward', () => {
    expect(Math.abs(totalPaid(rewards) - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });
});

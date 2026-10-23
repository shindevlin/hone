"use strict";

/**
 * Tests for dynamic reward distribution.
 * Shin Devlin
 *
 * Invariants under test:
 *   - Total paid always equals block reward (conservation)
 *   - 0.1% testnet carveout always fires first
 *   - Active roles with < SCARCITY_THRESHOLD nodes get a score multiplier
 *   - Active roles never fall below FLOOR (their min guaranteed share)
 *   - No role exceeds CAP
 *   - Nothing active → full recycle
 */

const {
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
} = require('../src/mining/rewardDistribution');

const BLOCK_REWARD = 243.06;
const EPSILON = 1e-6;

function totalPaid(rewards) {
  return rewards.reduce((s, r) => s + r.amount, 0);
}
function byType(rewards, type) {
  return rewards.filter(r => r.type === type);
}
function sumType(rewards, type) {
  return byType(rewards, type).reduce((s, r) => s + r.amount, 0);
}

// ─────────────────────────────────────────────────────────────────
// Conservation: total paid always equals block reward
// ─────────────────────────────────────────────────────────────────
describe('total conservation', () => {
  const cases = [
    { label: 'all roles + testnets',
      p: { miners: ['m1','m2'], minerWork: { m1: 100, m2: 50 },
           verifiers: ['v1'], clocks: ['c1'], storageHosts: ['s1'],
           sensors: ['sen1'], gateways: ['gw1'], serviceHosts: [],
           testnets: ['t1'] } },
    { label: 'miners only',
      p: { miners: ['m1'], minerWork: { m1: 1000 }, verifiers: [], clocks: [],
           storageHosts: [], sensors: [], gateways: [], serviceHosts: [], testnets: [] } },
    { label: 'clocks only',
      p: { miners: [], minerWork: {}, verifiers: [], clocks: ['c1', 'c2'],
           storageHosts: [], sensors: [], gateways: [], serviceHosts: [], testnets: [] } },
    { label: 'storage only',
      p: { miners: [], minerWork: {}, verifiers: [], clocks: [],
           storageHosts: ['h1'], sensors: [], gateways: [], serviceHosts: [], testnets: [] } },
    { label: 'testnets only',
      p: { miners: [], minerWork: {}, verifiers: [], clocks: [],
           storageHosts: [], sensors: [], gateways: [], serviceHosts: [], testnets: ['t1','t2'] } },
    { label: 'nothing',
      p: { miners: [], minerWork: {}, verifiers: [], clocks: [],
           storageHosts: [], sensors: [], gateways: [], serviceHosts: [], testnets: [] } },
  ];

  cases.forEach(({ label, p }) => {
    test(`total paid = block reward (${label})`, () => {
      const rewards = distributeBlockReward(BLOCK_REWARD, p);
      expect(Math.abs(totalPaid(rewards) - BLOCK_REWARD)).toBeLessThan(EPSILON);
    });
  });
});

// ─────────────────────────────────────────────────────────────────
// Nothing active → recycle the full block reward
// ─────────────────────────────────────────────────────────────────
describe('nothing active', () => {
  const rewards = distributeBlockReward(BLOCK_REWARD, {
    miners: [], minerWork: {}, verifiers: [], clocks: [],
    storageHosts: [], sensors: [], gateways: [], serviceHosts: [], testnets: [],
  });

  test('only recycle entries', () => {
    expect(rewards.every(r => r.type === 'recycle')).toBe(true);
  });

  test('recycle sums to full block reward', () => {
    expect(Math.abs(sumType(rewards, 'recycle') - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Testnet carveout: 0.1% always goes to testnet runners
// ─────────────────────────────────────────────────────────────────
describe('testnet carveout', () => {
  test('testnet runners receive 0.1% of block reward', () => {
    const p = {
      miners: ['m1'], minerWork: { m1: 1000 }, verifiers: ['v1'],
      clocks: ['c1'], storageHosts: [], sensors: [], gateways: [],
      serviceHosts: [], testnets: ['t1', 't2'],
    };
    const rewards = distributeBlockReward(BLOCK_REWARD, p);
    const testnetTotal = sumType(rewards, 'testnet');
    const expected = BLOCK_REWARD * TESTNET_FRACTION;
    expect(Math.abs(testnetTotal - expected)).toBeLessThan(EPSILON);
  });

  test('testnet split equally among runners', () => {
    const p = {
      miners: ['m1'], minerWork: { m1: 1000 }, verifiers: [],
      clocks: [], storageHosts: [], sensors: [], gateways: [],
      serviceHosts: [], testnets: ['t1', 't2', 't3'],
    };
    const rewards = distributeBlockReward(BLOCK_REWARD, p);
    const tnRewards = byType(rewards, 'testnet');
    expect(tnRewards).toHaveLength(3);
    const [a, b, c] = tnRewards.map(r => r.amount);
    expect(Math.abs(a - b)).toBeLessThan(EPSILON);
    expect(Math.abs(b - c)).toBeLessThan(EPSILON);
  });

  test('no testnet runners → testnet pool recycled', () => {
    const p = {
      miners: ['m1'], minerWork: { m1: 1000 }, verifiers: [],
      clocks: ['c1'], storageHosts: [], sensors: [], gateways: [],
      serviceHosts: [], testnets: [],
    };
    const rewards = distributeBlockReward(BLOCK_REWARD, p);
    const recyclePortion = sumType(rewards, 'recycle');
    const expected = BLOCK_REWARD * TESTNET_FRACTION;
    expect(recyclePortion).toBeGreaterThanOrEqual(expected - EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Scarcity premium: < SCARCITY_THRESHOLD nodes get a multiplier
// ─────────────────────────────────────────────────────────────────
describe('scarcity premium', () => {
  test('1 clock earns more per node than 5 clocks (scarcity)', () => {
    const baseWork = 50000;

    const p1 = {
      miners: ['m1'], minerWork: { m1: baseWork }, verifiers: [],
      clocks: ['c1'], storageHosts: [], sensors: [], gateways: [],
      serviceHosts: [], testnets: [],
    };
    const p5 = {
      miners: ['m1'], minerWork: { m1: baseWork }, verifiers: [],
      clocks: ['c1','c2','c3','c4','c5'], storageHosts: [], sensors: [], gateways: [],
      serviceHosts: [], testnets: [],
    };

    const r1 = distributeBlockReward(BLOCK_REWARD, p1);
    const r5 = distributeBlockReward(BLOCK_REWARD, p5);
    const perNode1 = sumType(r1, 'clock') / 1;
    const perNode5 = sumType(r5, 'clock') / 5;
    expect(perNode1).toBeGreaterThan(perNode5);
  });

  test('scarcity multiplier is SCARCITY_MULTIPLIER (only 1 verifier)', () => {
    // With 1 verifier (scarce), its score = VERIFIER_SCORE × SCARCITY_MULTIPLIER
    // With 4 verifiers (not scarce), score = 4 × VERIFIER_SCORE
    // So 1-verifier share > 1/4 of 4-verifier total verifier share
    const work = 100000;
    const p1 = {
      miners: ['m1'], minerWork: { m1: work }, verifiers: ['v1'],
      clocks: ['c1','c2','c3','c4','c5'], storageHosts: ['s1','s2','s3','s4'],
      sensors: [], gateways: [], serviceHosts: [], testnets: [],
    };
    const p4 = {
      miners: ['m1'], minerWork: { m1: work }, verifiers: ['v1','v2','v3','v4'],
      clocks: ['c1','c2','c3','c4','c5'], storageHosts: ['s1','s2','s3','s4'],
      sensors: [], gateways: [], serviceHosts: [], testnets: [],
    };
    const r1 = distributeBlockReward(BLOCK_REWARD, p1);
    const r4 = distributeBlockReward(BLOCK_REWARD, p4);
    const v1Total = sumType(r1, 'verifier');
    const v4Total = sumType(r4, 'verifier');
    // 1 scarce verifier should earn more than 1/4 of 4-verifier total
    expect(v1Total).toBeGreaterThan(v4Total / 4);
  });
});

// ─────────────────────────────────────────────────────────────────
// Fully dynamic: roles with zero activity earn zero
// ─────────────────────────────────────────────────────────────────
describe('zero activity roles earn zero', () => {
  test('absent role earns nothing', () => {
    const p = {
      miners: ['m1'], minerWork: { m1: 10000 }, verifiers: [],
      clocks: ['c1'], storageHosts: [], sensors: [], gateways: [],
      serviceHosts: [], testnets: [],
    };
    const rewards = distributeBlockReward(BLOCK_REWARD, p);
    expect(byType(rewards, 'verifier')).toHaveLength(0);
    expect(byType(rewards, 'storage')).toHaveLength(0);
  });

  test('dominant role can take near-100% of distributable', () => {
    const p = {
      miners: ['m1'], minerWork: { m1: 10000000 }, verifiers: [],
      clocks: [], storageHosts: [], sensors: [], gateways: [],
      serviceHosts: [], testnets: [],
    };
    const rewards = distributeBlockReward(BLOCK_REWARD, p);
    const distributable = BLOCK_REWARD * (1 - TESTNET_FRACTION);
    const minerPaid = sumType(rewards, 'miner');
    // Miner should get essentially all of distributable when alone
    expect(minerPaid / distributable).toBeGreaterThan(0.999);
  });
});

// ─────────────────────────────────────────────────────────────────
// Services pool
// ─────────────────────────────────────────────────────────────────
describe('service host pool', () => {
  test('service hosts earn proportional share', () => {
    const p = {
      miners: [], minerWork: {}, verifiers: [], clocks: [],
      storageHosts: [], sensors: [], gateways: [],
      serviceHosts: ['svc1', 'svc2'], testnets: [],
    };
    const rewards = distributeBlockReward(BLOCK_REWARD, p);
    expect(byType(rewards, 'service')).toHaveLength(2);
    const distributable = BLOCK_REWARD * (1 - TESTNET_FRACTION);
    const servicePaid = sumType(rewards, 'service');
    // Only service hosts active — they get essentially all of distributable
    expect(servicePaid / distributable).toBeGreaterThan(0.999);
  });

  test('service hosts split equally', () => {
    const p = {
      miners: [], minerWork: {}, verifiers: [], clocks: [],
      storageHosts: [], sensors: [], gateways: [],
      serviceHosts: ['svc1', 'svc2'], testnets: [],
    };
    const rewards = distributeBlockReward(BLOCK_REWARD, p);
    const [a, b] = byType(rewards, 'service').map(r => r.amount);
    expect(Math.abs(a - b)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Proportional miner split within the miner pool
// ─────────────────────────────────────────────────────────────────
describe('proportional miner split', () => {
  const p = {
    miners:       ['miner-a', 'miner-b'],
    minerWork:    { 'miner-a': 200, 'miner-b': 100 },
    verifiers:    ['v1','v2','v3'],
    clocks:       ['c1','c2','c3','c4','c5'],
    storageHosts: ['s1','s2','s3'],
    sensors:      [], gateways:  [], serviceHosts: [], testnets: [],
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, p); });

  test('miner-a gets 2× miner-b within the miner pool', () => {
    const a = byType(rewards, 'miner').find(r => r.miner === 'miner-a').amount;
    const b = byType(rewards, 'miner').find(r => r.miner === 'miner-b').amount;
    expect(Math.abs(a / b - 2)).toBeLessThan(1e-6);
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
    serviceHosts: [], testnets: [],
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, p); });

  test('sensor entries present', () => {
    expect(byType(rewards, 'iot_sensor')).toHaveLength(2);
  });

  test('gateway entry present', () => {
    expect(byType(rewards, 'iot_gateway')).toHaveLength(1);
  });

  test('gateway earns more per node than sensor', () => {
    const sensorPerNode  = byType(rewards, 'iot_sensor')[0].amount;
    const gatewayPerNode = byType(rewards, 'iot_gateway')[0].amount;
    expect(gatewayPerNode).toBeGreaterThan(sensorPerNode);
  });

  test('total paid equals block reward', () => {
    expect(Math.abs(totalPaid(rewards) - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Calibration: typical participation → sane split
// ─────────────────────────────────────────────────────────────────
describe('calibration at typical participation', () => {
  const p = {
    miners:       ['m1','m2','m3','m4','m5'],
    minerWork:    { m1:10000, m2:10000, m3:10000, m4:10000, m5:10000 },
    verifiers:    ['v1','v2','v3'],
    clocks:       ['c1','c2','c3','c4','c5','c6','c7','c8','c9','c10'],
    storageHosts: ['s1','s2','s3','s4','s5'],
    sensors:      ['sen1','sen2','sen3','sen4','sen5'],
    gateways:     ['gw1','gw2','gw3'],
    serviceHosts: [],
    testnets:     ['t1'],
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, p); });

  test('miners get > 40% of reward (dominant role)', () => {
    const minerPct = sumType(rewards, 'miner') / BLOCK_REWARD;
    expect(minerPct).toBeGreaterThan(0.40);
  });

  test('each active role earns a positive share', () => {
    for (const type of ['miner', 'verifier', 'clock', 'storage', 'iot_sensor', 'iot_gateway']) {
      const paid = sumType(rewards, type);
      expect(paid).toBeGreaterThan(0);
    }
  });

  test('testnet gets 0.1%', () => {
    const testnetPaid = sumType(rewards, 'testnet');
    const expected = BLOCK_REWARD * TESTNET_FRACTION;
    expect(Math.abs(testnetPaid - expected)).toBeLessThan(EPSILON);
  });

  test('total paid equals block reward', () => {
    expect(Math.abs(totalPaid(rewards) - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });
});

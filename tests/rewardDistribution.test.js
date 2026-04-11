"use strict";

/**
 * Tests for v2.13.4 five-pool block emission reward distribution.
 * Shin Devlin
 */

const {
  distributeBlockReward,
  MINER_POOL_PCT,
  VERIFIER_POOL_PCT,
  CLOCK_POOL_PCT,
  STORAGE_POOL_PCT,
  SERVICE_POOL_PCT,
} = require('../src/mining/rewardDistribution');

const BLOCK_REWARD = 243.06; // genesis epoch reward
const EPSILON = 1e-9;

// Helper: sum all reward amounts
function totalPaid(rewards) {
  return rewards.reduce((s, r) => s + r.amount, 0);
}

// Helper: find entries by type
function byType(rewards, type) {
  return rewards.filter(r => r.type === type);
}

// ─────────────────────────────────────────────────────────────────
// Full participation — all five roles active
// ─────────────────────────────────────────────────────────────────
describe('distributeBlockReward — all roles active', () => {
  const participants = {
    miners:       ['shindevlin', 'natoshisakamoto'],
    minerWork:    { shindevlin: 100, natoshisakamoto: 50 },
    verifiers:    ['verifier1'],
    clocks:       ['clock1'],
    storageHosts: ['storagehost1'],
    serviceHosts: ['servicehost1'],
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, participants); });

  test('no recycle entry when all pools are filled', () => {
    const recycleEntries = byType(rewards, 'recycle');
    expect(recycleEntries).toHaveLength(0);
  });

  test('total paid equals block reward', () => {
    expect(Math.abs(totalPaid(rewards) - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });

  test('miner pool equals 60% of block reward', () => {
    const minerTotal = byType(rewards, 'miner').reduce((s, r) => s + r.amount, 0);
    expect(Math.abs(minerTotal - BLOCK_REWARD * MINER_POOL_PCT)).toBeLessThan(EPSILON);
  });

  test('verifier pool equals 10% of block reward', () => {
    const vTotal = byType(rewards, 'verifier').reduce((s, r) => s + r.amount, 0);
    expect(Math.abs(vTotal - BLOCK_REWARD * VERIFIER_POOL_PCT)).toBeLessThan(EPSILON);
  });

  test('clock pool equals 5% of block reward', () => {
    const cTotal = byType(rewards, 'clock').reduce((s, r) => s + r.amount, 0);
    expect(Math.abs(cTotal - BLOCK_REWARD * CLOCK_POOL_PCT)).toBeLessThan(EPSILON);
  });

  test('storage pool equals 15% of block reward', () => {
    const sTotal = byType(rewards, 'storage').reduce((s, r) => s + r.amount, 0);
    expect(Math.abs(sTotal - BLOCK_REWARD * STORAGE_POOL_PCT)).toBeLessThan(EPSILON);
  });

  test('service pool equals 10% of block reward', () => {
    const svTotal = byType(rewards, 'service').reduce((s, r) => s + r.amount, 0);
    expect(Math.abs(svTotal - BLOCK_REWARD * SERVICE_POOL_PCT)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Only clocks active
// ─────────────────────────────────────────────────────────────────
describe('distributeBlockReward — only clocks active', () => {
  const participants = {
    miners:       [],
    minerWork:    {},
    verifiers:    [],
    clocks:       ['clock1', 'clock2'],
    storageHosts: [],
    serviceHosts: [],
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, participants); });

  test('clock entries present', () => {
    expect(byType(rewards, 'clock')).toHaveLength(2);
  });

  test('recycle entry present', () => {
    expect(byType(rewards, 'recycle')).toHaveLength(1);
  });

  test('recycle amount is 95% of block reward', () => {
    const recycleAmt = byType(rewards, 'recycle')[0].amount;
    const expected = BLOCK_REWARD * (MINER_POOL_PCT + VERIFIER_POOL_PCT + STORAGE_POOL_PCT + SERVICE_POOL_PCT);
    expect(Math.abs(recycleAmt - expected)).toBeLessThan(EPSILON);
  });

  test('clock pool is 5% of block reward', () => {
    const cTotal = byType(rewards, 'clock').reduce((s, r) => s + r.amount, 0);
    expect(Math.abs(cTotal - BLOCK_REWARD * CLOCK_POOL_PCT)).toBeLessThan(EPSILON);
  });

  test('total paid equals block reward', () => {
    expect(Math.abs(totalPaid(rewards) - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Only storage active
// ─────────────────────────────────────────────────────────────────
describe('distributeBlockReward — only storage active', () => {
  const participants = {
    miners:       [],
    minerWork:    {},
    verifiers:    [],
    clocks:       [],
    storageHosts: ['storagehost1'],
    serviceHosts: [],
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, participants); });

  test('storage entry present', () => {
    expect(byType(rewards, 'storage')).toHaveLength(1);
  });

  test('storage amount is 15% of block reward', () => {
    expect(Math.abs(byType(rewards, 'storage')[0].amount - BLOCK_REWARD * STORAGE_POOL_PCT)).toBeLessThan(EPSILON);
  });

  test('recycle is 85% of block reward', () => {
    const expected = BLOCK_REWARD * (MINER_POOL_PCT + VERIFIER_POOL_PCT + CLOCK_POOL_PCT + SERVICE_POOL_PCT);
    expect(Math.abs(byType(rewards, 'recycle')[0].amount - expected)).toBeLessThan(EPSILON);
  });

  test('total paid equals block reward', () => {
    expect(Math.abs(totalPaid(rewards) - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Miners + clocks only (typical busy chain)
// ─────────────────────────────────────────────────────────────────
describe('distributeBlockReward — miners + clocks (typical busy chain)', () => {
  const participants = {
    miners:       ['shindevlin'],
    minerWork:    { shindevlin: 200 },
    verifiers:    [],
    clocks:       ['clock1'],
    storageHosts: [],
    serviceHosts: [],
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, participants); });

  test('65% claimed: miner 60% + clock 5%', () => {
    const claimed = byType(rewards, 'miner').reduce((s, r) => s + r.amount, 0)
                  + byType(rewards, 'clock').reduce((s, r) => s + r.amount, 0);
    expect(Math.abs(claimed - BLOCK_REWARD * 0.65)).toBeLessThan(EPSILON);
  });

  test('35% recycled', () => {
    const recycled = byType(rewards, 'recycle')[0].amount;
    expect(Math.abs(recycled - BLOCK_REWARD * 0.35)).toBeLessThan(EPSILON);
  });

  test('total paid equals block reward', () => {
    expect(Math.abs(totalPaid(rewards) - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Proportional miner split: 100 work + 50 work → 2:1 ratio
// ─────────────────────────────────────────────────────────────────
describe('distributeBlockReward — proportional miner split', () => {
  const participants = {
    miners:       ['miner-a', 'miner-b'],
    minerWork:    { 'miner-a': 100, 'miner-b': 50 },
    verifiers:    [],
    clocks:       [],
    storageHosts: [],
    serviceHosts: [],
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, participants); });

  test('miner-a gets 2x what miner-b gets', () => {
    const a = rewards.find(r => r.miner === 'miner-a');
    const b = rewards.find(r => r.miner === 'miner-b');
    expect(a).toBeDefined();
    expect(b).toBeDefined();
    expect(Math.abs(a.amount / b.amount - 2)).toBeLessThan(EPSILON);
  });

  test('miner pool total is 60% of block reward', () => {
    const minerTotal = byType(rewards, 'miner').reduce((s, r) => s + r.amount, 0);
    expect(Math.abs(minerTotal - BLOCK_REWARD * MINER_POOL_PCT)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Multiple storage hosts: equal split of storage pool
// ─────────────────────────────────────────────────────────────────
describe('distributeBlockReward — multiple storage hosts', () => {
  const hosts = ['host-a', 'host-b', 'host-c'];
  const participants = {
    miners:       [],
    minerWork:    {},
    verifiers:    [],
    clocks:       [],
    storageHosts: hosts,
    serviceHosts: [],
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, participants); });

  test('three storage entries', () => {
    expect(byType(rewards, 'storage')).toHaveLength(3);
  });

  test('each storage host gets equal share', () => {
    const amounts = byType(rewards, 'storage').map(r => r.amount);
    const expected = BLOCK_REWARD * STORAGE_POOL_PCT / 3;
    amounts.forEach(a => expect(Math.abs(a - expected)).toBeLessThan(EPSILON));
  });

  test('storage pool total is 15%', () => {
    const sTotal = byType(rewards, 'storage').reduce((s, r) => s + r.amount, 0);
    expect(Math.abs(sTotal - BLOCK_REWARD * STORAGE_POOL_PCT)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Multiple service hosts: equal split of service pool
// ─────────────────────────────────────────────────────────────────
describe('distributeBlockReward — multiple service hosts', () => {
  const hosts = ['svchost-1', 'svchost-2'];
  const participants = {
    miners:       [],
    minerWork:    {},
    verifiers:    [],
    clocks:       [],
    storageHosts: [],
    serviceHosts: hosts,
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, participants); });

  test('two service entries', () => {
    expect(byType(rewards, 'service')).toHaveLength(2);
  });

  test('each service host gets equal share', () => {
    const amounts = byType(rewards, 'service').map(r => r.amount);
    const expected = BLOCK_REWARD * SERVICE_POOL_PCT / 2;
    amounts.forEach(a => expect(Math.abs(a - expected)).toBeLessThan(EPSILON));
  });
});

// ─────────────────────────────────────────────────────────────────
// Zero participants in all roles: 100% recycle
// ─────────────────────────────────────────────────────────────────
describe('distributeBlockReward — zero participants in all roles', () => {
  const participants = {
    miners:       [],
    minerWork:    {},
    verifiers:    [],
    clocks:       [],
    storageHosts: [],
    serviceHosts: [],
  };

  let rewards;
  beforeAll(() => { rewards = distributeBlockReward(BLOCK_REWARD, participants); });

  test('single recycle entry', () => {
    expect(rewards).toHaveLength(1);
    expect(rewards[0].type).toBe('recycle');
    expect(rewards[0].miner).toBe('btcpc_recycle');
  });

  test('recycle amount equals full block reward', () => {
    expect(Math.abs(rewards[0].amount - BLOCK_REWARD)).toBeLessThan(EPSILON);
  });
});

// ─────────────────────────────────────────────────────────────────
// Floating-point integrity: totals must sum to block reward
// ─────────────────────────────────────────────────────────────────
describe('distributeBlockReward — floating-point sum integrity', () => {
  const scenarios = [
    {
      label: 'all active',
      participants: {
        miners: ['m1', 'm2', 'm3'],
        minerWork: { m1: 75, m2: 150, m3: 25 },
        verifiers: ['v1', 'v2'],
        clocks: ['c1', 'c2', 'c3'],
        storageHosts: ['sh1'],
        serviceHosts: ['sv1', 'sv2'],
      },
    },
    {
      label: 'no storage or service',
      participants: {
        miners: ['m1'],
        minerWork: { m1: 1 },
        verifiers: ['v1'],
        clocks: ['c1'],
        storageHosts: [],
        serviceHosts: [],
      },
    },
    {
      label: 'all empty',
      participants: {
        miners: [], minerWork: {}, verifiers: [], clocks: [], storageHosts: [], serviceHosts: [],
      },
    },
  ];

  for (const scenario of scenarios) {
    test(`${scenario.label}: sum within 1e-9 of block reward`, () => {
      const rewards = distributeBlockReward(BLOCK_REWARD, scenario.participants);
      expect(Math.abs(totalPaid(rewards) - BLOCK_REWARD)).toBeLessThan(EPSILON);
    });
  }
});

// ─────────────────────────────────────────────────────────────────
// Recycle entry: only created when recycledAmount > 0
// ─────────────────────────────────────────────────────────────────
describe('distributeBlockReward — recycle entry conditional', () => {
  test('no recycle entry when all roles have participants', () => {
    const rewards = distributeBlockReward(BLOCK_REWARD, {
      miners: ['m1'],
      minerWork: { m1: 1 },
      verifiers: ['v1'],
      clocks: ['c1'],
      storageHosts: ['sh1'],
      serviceHosts: ['sv1'],
    });
    expect(byType(rewards, 'recycle')).toHaveLength(0);
  });

  test('recycle entry present when at least one role is empty', () => {
    const rewards = distributeBlockReward(BLOCK_REWARD, {
      miners: ['m1'],
      minerWork: { m1: 1 },
      verifiers: [],
      clocks: ['c1'],
      storageHosts: [],
      serviceHosts: [],
    });
    expect(byType(rewards, 'recycle')).toHaveLength(1);
    expect(byType(rewards, 'recycle')[0].amount).toBeGreaterThan(0);
  });
});

// ─────────────────────────────────────────────────────────────────
// stateStore.getStorageHostsForEpoch — unit tests
// ─────────────────────────────────────────────────────────────────
describe('stateStore.getStorageHostsForEpoch', () => {
  let stateStore;

  beforeAll(() => {
    stateStore = require('../src/chain/stateStore');
  });

  test('returns empty array when no heartbeats recorded', () => {
    const hosts = stateStore.getStorageHostsForEpoch(9999);
    expect(hosts).toEqual([]);
  });

  test('returns hosts that heartbeated in the given epoch via applyEntry', () => {
    // Inject a STORAGE_HEARTBEAT via applyEntry (the canonical mutation path)
    stateStore.applyEntry({
      type: 'ACCOUNT_CREATE',
      from: 'testhostx',
      epoch: 5000,
      metadata: {},
    });
    stateStore.applyEntry({
      type: 'STORAGE_HEARTBEAT',
      from: 'testhostx',
      epoch: 5000,
      blob_data: { cids: [], capacity_used_gb: 0 },
    });

    const hosts = stateStore.getStorageHostsForEpoch(5000);
    expect(hosts).toContain('testhostx');
  });

  test('does not return hosts from a different epoch', () => {
    const hosts = stateStore.getStorageHostsForEpoch(5001);
    expect(hosts).not.toContain('testhostx');
  });
});

// ─────────────────────────────────────────────────────────────────
// serviceRegistry._getHeartbeatsForEpoch — unit tests
// ─────────────────────────────────────────────────────────────────
describe('serviceRegistry._getHeartbeatsForEpoch', () => {
  let serviceRegistry;

  beforeAll(() => {
    serviceRegistry = require('../src/services/serviceRegistry');
    serviceRegistry.resetForTests();

    // Deploy a service so heartbeat() won't throw
    serviceRegistry.deploy('deployer1', 'deployer1/test-svc', {
      type: 'http',
      binary_cid: 'a'.repeat(64),
    }, { epoch: 1 });

    serviceRegistry.heartbeat('host-alpha', 'deployer1/test-svc', { epoch: 42 });
    serviceRegistry.heartbeat('host-beta',  'deployer1/test-svc', { epoch: 43 });
  });

  test('returns heartbeats for exact epoch', () => {
    const hbs = serviceRegistry._getHeartbeatsForEpoch(42);
    expect(hbs.map(h => h.host)).toContain('host-alpha');
    expect(hbs.map(h => h.host)).not.toContain('host-beta');
  });

  test('returns empty array for epoch with no heartbeats', () => {
    const hbs = serviceRegistry._getHeartbeatsForEpoch(999);
    expect(hbs).toHaveLength(0);
  });

  afterAll(() => {
    serviceRegistry.resetForTests();
  });
});

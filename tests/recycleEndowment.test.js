"use strict";

/**
 * btcpc_recycle Perpetual Endowment — v3.3
 * Shin Devlin
 *
 * Tests for Phase 2 activation, recycle rate computation, and
 * distribution from btcpc_recycle balance.
 */

const stateStore = require('../src/chain/stateStore');
const { computeRewards, RECYCLE_ACCOUNT, TOTAL_SUPPLY } = require('../src/chain/rewardEngine');

function round(n) {
  return parseFloat(Number(n).toFixed(10));
}

function creditRecycle(amount) {
  stateStore.applyEntry({
    type: 'MINING_REWARD',
    to: RECYCLE_ACCOUNT,
    amount,
    epoch: 0,
    reward_source: 'reserve',
  });
}

function distributeSupply(amount) {
  // Drive totalSupplyDistributed up without touching recycle balance
  stateStore.applyEntry({
    type: 'MINING_REWARD',
    to: 'shindevlin',
    amount,
    epoch: 0,
    reward_source: 'mining',
  });
}

beforeEach(() => {
  stateStore.resetAll();
  stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'shindevlin', epoch: 0, account_data: { public_keys: {}, chain_addresses: {} } });
  stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'miner1', epoch: 0, account_data: { public_keys: {}, chain_addresses: {} } });
  stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'miner2', epoch: 0, account_data: { public_keys: {}, chain_addresses: {} } });
});

// ──────────────────────────────────────────────────────────────────────────────
// TOTAL_SUPPLY constant
// ──────────────────────────────────────────────────────────────────────────────

describe('TOTAL_SUPPLY constant', () => {
  test('is 42,000,000', () => {
    expect(TOTAL_SUPPLY).toBe(42000000);
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// Phase 2 flag — stateStore
// ──────────────────────────────────────────────────────────────────────────────

describe('stateStore.isPhase2()', () => {
  test('Phase 2 is false initially', () => {
    expect(stateStore.isPhase2()).toBe(false);
    expect(stateStore.getTotalSupplyDistributed()).toBe(0);
  });

  test('Phase 2 is false below 42M', () => {
    distributeSupply(41999999);
    expect(stateStore.isPhase2()).toBe(false);
  });

  test('Phase 2 is true at exactly 42M', () => {
    distributeSupply(42000000);
    expect(stateStore.isPhase2()).toBe(true);
  });

  test('Phase 2 is true above 42M', () => {
    distributeSupply(45000000);
    expect(stateStore.isPhase2()).toBe(true);
  });

  test('recycle rewards do not count toward totalSupplyDistributed', () => {
    creditRecycle(5000000);
    distributeSupply(41000000);
    expect(stateStore.isPhase2()).toBe(false);
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// setRecycleRate / getRecycleRate
// ──────────────────────────────────────────────────────────────────────────────

describe('setRecycleRate / getRecycleRate', () => {
  test('getRecycleRate returns null before activation', () => {
    expect(stateStore.getRecycleRate()).toBeNull();
  });

  test('setRecycleRate stores rate and marks activated', () => {
    stateStore.setRecycleRate(0.001, 5000);
    expect(stateStore.getRecycleRate()).toBeCloseTo(0.001);
    expect(stateStore.isPhase2Activated()).toBe(true);
    expect(stateStore.getPhase2ActivationEpoch()).toBe(5000);
  });

  test('setRecycleRate rejects zero', () => {
    stateStore.setRecycleRate(0, 100);
    expect(stateStore.getRecycleRate()).toBeNull();
    expect(stateStore.isPhase2Activated()).toBe(false);
  });

  test('setRecycleRate rejects negative', () => {
    stateStore.setRecycleRate(-1, 100);
    expect(stateStore.getRecycleRate()).toBeNull();
  });

  test('rate cleared on resetAll', () => {
    stateStore.setRecycleRate(0.005, 100);
    stateStore.resetAll();
    expect(stateStore.getRecycleRate()).toBeNull();
    expect(stateStore.isPhase2Activated()).toBe(false);
    expect(stateStore.getPhase2ActivationEpoch()).toBeNull();
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// computeRewards — Phase 1 (normal operation, stateStore injected)
// ──────────────────────────────────────────────────────────────────────────────

describe('computeRewards — Phase 1 (stateStore injected, below 42M)', () => {
  let mockStore;

  beforeEach(() => {
    mockStore = {
      isPhase2: () => false,
      isPhase2Activated: () => false,
      getBalance: () => 0,
      getRecycleRate: () => null,
      setRecycleRate: () => {},
    };
  });

  test('uses blockReward unchanged in Phase 1', () => {
    const { summary } = computeRewards({
      epochNumber: 1,
      blockReward: 100,
      computeProofs: [{ node_id: 'miner1', work_value: 1, result_hash: 'abc' }],
      stateStore: mockStore,
    });
    expect(summary.phase2).toBe(false);
    expect(summary.effective_block_reward).toBe(100);
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// computeRewards — Phase 2 activation on first epoch
// ──────────────────────────────────────────────────────────────────────────────

describe('computeRewards — Phase 2 first epoch (r computation)', () => {
  test('on first Phase 2 epoch, r = blockReward / recycleBalance', () => {
    const rateStore = { r: null, activated: false };
    const mockStore = {
      isPhase2: () => true,
      isPhase2Activated: () => false, // first Phase 2 epoch
      getBalance: (account) => account === RECYCLE_ACCOUNT ? 10000 : 0,
      getRecycleRate: () => rateStore.r,
      setRecycleRate: (r, epoch) => {
        rateStore.r = r;
        rateStore.activated = true;
        rateStore.epoch = epoch;
      },
    };

    const blockReward = 243.06;
    const recycleBalance = 10000;
    const expectedR = round(blockReward / recycleBalance);

    const { summary, phase2 } = computeRewards({
      epochNumber: 50000,
      blockReward,
      computeProofs: [{ node_id: 'miner1', work_value: 1, result_hash: 'abc' }],
      stateStore: mockStore,
    });

    expect(phase2).toBe(true);
    expect(summary.phase2).toBe(true);
    expect(rateStore.activated).toBe(true);
    expect(rateStore.r).toBeCloseTo(expectedR);
    // effectiveBlockReward = recycleBalance * r = recycleBalance * (blockReward/recycleBalance) = blockReward
    expect(summary.effective_block_reward).toBeCloseTo(blockReward, 5);
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// computeRewards — Phase 2 subsequent epochs (r already set)
// ──────────────────────────────────────────────────────────────────────────────

describe('computeRewards — Phase 2 subsequent epochs', () => {
  test('effective reward = recycleBalance × r', () => {
    const RECYCLE_BALANCE = 8000;
    const STORED_R = 0.03;

    const mockStore = {
      isPhase2: () => true,
      isPhase2Activated: () => true, // already activated
      getBalance: (account) => account === RECYCLE_ACCOUNT ? RECYCLE_BALANCE : 0,
      getRecycleRate: () => STORED_R,
      setRecycleRate: () => {},
    };

    const { summary, phase2 } = computeRewards({
      epochNumber: 50001,
      blockReward: 0, // Phase 1 schedule would produce 0, but Phase 2 overrides
      computeProofs: [{ node_id: 'miner1', work_value: 1, result_hash: 'abc' }],
      stateStore: mockStore,
    });

    const expectedEffective = round(RECYCLE_BALANCE * STORED_R);
    expect(phase2).toBe(true);
    expect(summary.effective_block_reward).toBeCloseTo(expectedEffective, 5);
  });

  test('when recycle balance is zero, effective reward is zero', () => {
    const mockStore = {
      isPhase2: () => true,
      isPhase2Activated: () => true,
      getBalance: () => 0,
      getRecycleRate: () => 0.05,
      setRecycleRate: () => {},
    };

    const { summary } = computeRewards({
      epochNumber: 50002,
      blockReward: 100,
      computeProofs: [],
      stateStore: mockStore,
    });

    expect(summary.phase2).toBe(true);
    expect(summary.effective_block_reward).toBe(0);
  });

  test('when r is null/zero, effective reward is zero (safe fallback)', () => {
    const mockStore = {
      isPhase2: () => true,
      isPhase2Activated: () => true,
      getBalance: () => 5000,
      getRecycleRate: () => null,
      setRecycleRate: () => {},
    };

    const { summary } = computeRewards({
      epochNumber: 50003,
      blockReward: 100,
      computeProofs: [],
      stateStore: mockStore,
    });

    expect(summary.phase2).toBe(true);
    expect(summary.effective_block_reward).toBe(0);
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// Phase 2 reward distribution uses same pool splits as Phase 1
// ──────────────────────────────────────────────────────────────────────────────

describe('computeRewards — Phase 2 uses same pool percentages', () => {
  test('mining pool gets 55% of effectiveBlockReward in Phase 2', () => {
    const RECYCLE_BALANCE = 5000;
    const STORED_R = 0.02;
    const expectedEffective = round(RECYCLE_BALANCE * STORED_R); // 100

    const mockStore = {
      isPhase2: () => true,
      isPhase2Activated: () => true,
      getBalance: () => RECYCLE_BALANCE,
      getRecycleRate: () => STORED_R,
      setRecycleRate: () => {},
    };

    const { rewards, summary } = computeRewards({
      epochNumber: 60000,
      blockReward: 0,
      computeProofs: [{ node_id: 'miner1', work_value: 100, result_hash: 'abc' }],
      verifiers: [],
      clockNodes: [],
      storageHosts: [],
      activeSensors: [],
      serviceHosts: [],
      stateStore: mockStore,
    });

    const miningRewards = rewards.filter(r => r.type === 'MINING_REWARD' && r.to !== RECYCLE_ACCOUNT);
    const miningTotal = miningRewards.reduce((s, r) => s + r.amount, 0);
    expect(miningTotal).toBeCloseTo(expectedEffective * 0.55, 4);
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// computeRewards — no stateStore available (isolation fallback)
// ──────────────────────────────────────────────────────────────────────────────

describe('computeRewards — stateStore unavailable fallback', () => {
  test('uses blockReward directly when stateStore throws', () => {
    const badStore = {
      isPhase2: () => { throw new Error('unavailable'); },
    };
    const { summary } = computeRewards({
      epochNumber: 1,
      blockReward: 50,
      computeProofs: [{ node_id: 'miner1', work_value: 1, result_hash: 'abc' }],
      stateStore: badStore,
    });
    expect(summary.phase2).toBe(false);
    expect(summary.effective_block_reward).toBe(50);
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// btcpc_recycle balance integration (live stateStore)
// ──────────────────────────────────────────────────────────────────────────────

describe('btcpc_recycle balance in live stateStore', () => {
  test('recycle account accumulates from reserve pool entries', () => {
    creditRecycle(500);
    expect(stateStore.getBalance(RECYCLE_ACCOUNT, 'BTCPC')).toBe(500);
  });

  test('multiple credits accumulate correctly', () => {
    creditRecycle(100);
    creditRecycle(200);
    creditRecycle(300);
    expect(stateStore.getBalance(RECYCLE_ACCOUNT, 'BTCPC')).toBeCloseTo(600, 5);
  });

  test('recycle balance does not contribute to totalSupplyDistributed', () => {
    creditRecycle(1000);
    expect(stateStore.getTotalSupplyDistributed()).toBe(0);
  });

  test('miner rewards do contribute to totalSupplyDistributed', () => {
    distributeSupply(1000);
    expect(stateStore.getTotalSupplyDistributed()).toBe(1000);
  });
});

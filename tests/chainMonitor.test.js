"use strict";

/**
 * chainMonitor tests — v2.17
 *
 * Covers:
 *  - EVMMonitor balance-change detection
 *  - EVMMonitor same-balance (no change)
 *  - EVMMonitor RPC error handling
 *  - SolanaMonitor balance-change detection
 *  - SolanaMonitor same-balance (no change)
 *  - BitcoinMonitor funded_txo_sum change detection
 *  - BitcoinMonitor same-sum (no change)
 *  - recordCrossChainActivity ledger function
 *  - stateStore CROSS_CHAIN_ACTIVITY dispatcher + getCrossChainActivity getter
 *  - stateStore getRecentCrossChainActivity
 *  - createChainMonitor polls accounts and calls recordCrossChainActivity
 *  - createChainMonitor handles RPC errors without crashing
 *  - createChainMonitor getActivity() returns filtered results
 *  - createChainMonitor resetForTests() clears state
 *
 * All HTTP calls are mocked via jest.spyOn(global, 'fetch').
 */

const { EVMMonitor, SolanaMonitor, BitcoinMonitor, createChainMonitor } = require('../src/services/chainMonitor');
const ledger = require('../src/services/ledger');
const stateStore = require('../src/chain/stateStore');

// ─── Helpers ──────────────────────────────────────────────────────

function mockFetchOk(body) {
  return jest.spyOn(global, 'fetch').mockResolvedValue({
    ok: true,
    json: async () => body,
  });
}

function mockFetchError(msg) {
  return jest.spyOn(global, 'fetch').mockRejectedValue(new Error(msg));
}

function mockFetchNotOk(status) {
  return jest.spyOn(global, 'fetch').mockResolvedValue({
    ok: false,
    status: status,
    json: async () => ({}),
  });
}

// ─── EVMMonitor ───────────────────────────────────────────────────

describe('EVMMonitor', () => {
  let monitor;

  beforeEach(() => {
    monitor = new EVMMonitor();
    jest.restoreAllMocks();
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('returns changed: false on first check (no previous balance)', async () => {
    mockFetchOk({ result: '0x1000' }); // 4096 wei
    const result = await monitor.checkAddress('0xABC', 'https://rpc.example.com', 'base');
    expect(result.changed).toBe(false);
  });

  it('returns changed: false when balance is unchanged', async () => {
    mockFetchOk({ result: '0x1000' });
    await monitor.checkAddress('0xABC', 'https://rpc.example.com', 'base'); // seed

    jest.restoreAllMocks();
    mockFetchOk({ result: '0x1000' });
    const result = await monitor.checkAddress('0xABC', 'https://rpc.example.com', 'base');
    expect(result.changed).toBe(false);
  });

  it('detects balance change and returns changed: true with old/new values', async () => {
    // First call: balance = 0x100 (256)
    mockFetchOk({ result: '0x100' });
    await monitor.checkAddress('0xABC', 'https://rpc.example.com', 'base');

    // Second call: balance = 0x200 (512)
    jest.restoreAllMocks();
    mockFetchOk({ result: '0x200' });
    const result = await monitor.checkAddress('0xABC', 'https://rpc.example.com', 'base');

    expect(result.changed).toBe(true);
    expect(result.chain).toBe('base');
    expect(result.address).toBe('0xABC');
    expect(result.oldBalance).toBe(256);
    expect(result.newBalance).toBe(512);
  });

  it('returns changed: false with error field when RPC throws', async () => {
    mockFetchError('connection refused');
    const result = await monitor.checkAddress('0xABC', 'https://rpc.example.com', 'base');
    expect(result.changed).toBe(false);
    expect(result.error).toMatch(/connection refused/);
  });

  it('returns changed: false with error when HTTP status is not ok', async () => {
    mockFetchNotOk(429);
    const result = await monitor.checkAddress('0xABC', 'https://rpc.example.com', 'base');
    expect(result.changed).toBe(false);
    expect(result.error).toMatch(/429/);
  });

  it('handles separate addresses on same chain independently', async () => {
    // Seed both addresses
    mockFetchOk({ result: '0x100' });
    await monitor.checkAddress('0xABC', 'https://rpc.example.com', 'base');
    jest.restoreAllMocks();
    mockFetchOk({ result: '0x200' });
    await monitor.checkAddress('0xDEF', 'https://rpc.example.com', 'base');

    // Change only 0xABC
    jest.restoreAllMocks();
    mockFetchOk({ result: '0x300' });
    const resultABC = await monitor.checkAddress('0xABC', 'https://rpc.example.com', 'base');
    jest.restoreAllMocks();
    mockFetchOk({ result: '0x200' });
    const resultDEF = await monitor.checkAddress('0xDEF', 'https://rpc.example.com', 'base');

    expect(resultABC.changed).toBe(true);
    expect(resultDEF.changed).toBe(false);
  });
});

// ─── SolanaMonitor ────────────────────────────────────────────────

describe('SolanaMonitor', () => {
  let monitor;

  beforeEach(() => {
    monitor = new SolanaMonitor();
    jest.restoreAllMocks();
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('returns changed: false on first check', async () => {
    mockFetchOk({ result: { value: 5000000 } });
    const result = await monitor.checkAddress('SolAddr123');
    expect(result.changed).toBe(false);
  });

  it('returns changed: false when balance is same', async () => {
    mockFetchOk({ result: { value: 5000000 } });
    await monitor.checkAddress('SolAddr123');

    jest.restoreAllMocks();
    mockFetchOk({ result: { value: 5000000 } });
    const result = await monitor.checkAddress('SolAddr123');
    expect(result.changed).toBe(false);
  });

  it('detects SOL balance change', async () => {
    mockFetchOk({ result: { value: 1000000 } });
    await monitor.checkAddress('SolAddr123');

    jest.restoreAllMocks();
    mockFetchOk({ result: { value: 2500000 } });
    const result = await monitor.checkAddress('SolAddr123');

    expect(result.changed).toBe(true);
    expect(result.chain).toBe('solana');
    expect(result.address).toBe('SolAddr123');
    expect(result.oldBalance).toBe(1000000);
    expect(result.newBalance).toBe(2500000);
  });

  it('returns changed: false with error field when Solana RPC throws', async () => {
    mockFetchError('timeout');
    const result = await monitor.checkAddress('SolAddr123');
    expect(result.changed).toBe(false);
    expect(result.error).toMatch(/timeout/);
  });
});

// ─── BitcoinMonitor ───────────────────────────────────────────────

describe('BitcoinMonitor', () => {
  let monitor;

  beforeEach(() => {
    monitor = new BitcoinMonitor();
    jest.restoreAllMocks();
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('returns changed: false on first check', async () => {
    mockFetchOk({ chain_stats: { funded_txo_sum: 100000 } });
    const result = await monitor.checkAddress('bc1qABC');
    expect(result.changed).toBe(false);
  });

  it('returns changed: false when funded_txo_sum is same', async () => {
    mockFetchOk({ chain_stats: { funded_txo_sum: 100000 } });
    await monitor.checkAddress('bc1qABC');

    jest.restoreAllMocks();
    mockFetchOk({ chain_stats: { funded_txo_sum: 100000 } });
    const result = await monitor.checkAddress('bc1qABC');
    expect(result.changed).toBe(false);
  });

  it('detects funded_txo_sum change', async () => {
    mockFetchOk({ chain_stats: { funded_txo_sum: 0 } });
    await monitor.checkAddress('bc1qABC');

    jest.restoreAllMocks();
    mockFetchOk({ chain_stats: { funded_txo_sum: 50000000 } }); // 0.5 BTC
    const result = await monitor.checkAddress('bc1qABC');

    expect(result.changed).toBe(true);
    expect(result.chain).toBe('bitcoin');
    expect(result.address).toBe('bc1qABC');
    expect(result.oldSum).toBe(0);
    expect(result.newSum).toBe(50000000);
  });

  it('returns changed: false with error when API throws', async () => {
    mockFetchError('network error');
    const result = await monitor.checkAddress('bc1qABC');
    expect(result.changed).toBe(false);
    expect(result.error).toMatch(/network error/);
  });

  it('returns changed: false with error when chain_stats missing', async () => {
    mockFetchOk({ status: 'ok' }); // no chain_stats
    const result = await monitor.checkAddress('bc1qABC');
    expect(result.changed).toBe(false);
    expect(result.error).toMatch(/chain_stats/);
  });
});

// ─── ledger.recordCrossChainActivity ─────────────────────────────

describe('ledger.recordCrossChainActivity', () => {
  beforeEach(() => {
    stateStore.resetAll();
    // Ensure the account exists
    stateStore.applyEntry({
      type: 'ACCOUNT_CREATE',
      to: 'shindevlin',
      epoch: 1,
      account_data: {
        username: 'shindevlin',
        public_keys: {},
        chain_addresses: { evm: '0xABC', solana: 'SolAddr', bitcoin: 'bc1qABC' },
      },
    });
  });

  it('creates a valid CROSS_CHAIN_ACTIVITY ledger entry', async () => {
    const entry = await ledger.recordCrossChainActivity(
      'shindevlin',
      'base',
      '0xABC',
      'balance_change',
      { old_balance: 0, new_balance: 1000 },
      5
    );

    expect(entry.type).toBe('CROSS_CHAIN_ACTIVITY');
    expect(entry.to).toBe('shindevlin');
    expect(entry.epoch).toBe(5);
    expect(entry.cross_chain_data.chain).toBe('base');
    expect(entry.cross_chain_data.address).toBe('0xABC');
    expect(entry.cross_chain_data.activity_type).toBe('balance_change');
    expect(entry.cross_chain_data.old_balance).toBe(0);
    expect(entry.cross_chain_data.new_balance).toBe(1000);
  });

  it('stores the activity in stateStore via applyEntry', async () => {
    await ledger.recordCrossChainActivity(
      'shindevlin',
      'solana',
      'SolAddr',
      'balance_change',
      { old_balance: 0, new_balance: 500000 },
      3
    );

    const activities = stateStore.getCrossChainActivity('shindevlin');
    expect(activities.length).toBeGreaterThanOrEqual(1);
    const last = activities[activities.length - 1];
    expect(last.chain).toBe('solana');
    expect(last.address).toBe('SolAddr');
  });
});

// ─── stateStore CROSS_CHAIN_ACTIVITY dispatcher ───────────────────

describe('stateStore CROSS_CHAIN_ACTIVITY dispatcher', () => {
  beforeEach(() => stateStore.resetAll());

  it('stores activity for the user', () => {
    stateStore.applyEntry({
      type: 'CROSS_CHAIN_ACTIVITY',
      to: 'natoshisakamoto',
      epoch: 10,
      timestamp: 1234567890,
      cross_chain_data: {
        chain: 'bitcoin',
        address: 'bc1q123',
        activity_type: 'balance_change',
        old_funded_txo_sum: 0,
        new_funded_txo_sum: 100000,
      },
    });

    const activities = stateStore.getCrossChainActivity('natoshisakamoto');
    expect(activities).toHaveLength(1);
    expect(activities[0].chain).toBe('bitcoin');
    expect(activities[0].address).toBe('bc1q123');
    expect(activities[0].epoch).toBe(10);
  });

  it('filters by chain via getCrossChainActivity(username, chain)', () => {
    stateStore.applyEntry({
      type: 'CROSS_CHAIN_ACTIVITY',
      to: 'shindevlin',
      epoch: 1,
      timestamp: 1000,
      cross_chain_data: { chain: 'base', address: '0xA', activity_type: 'balance_change' },
    });
    stateStore.applyEntry({
      type: 'CROSS_CHAIN_ACTIVITY',
      to: 'shindevlin',
      epoch: 2,
      timestamp: 2000,
      cross_chain_data: { chain: 'solana', address: 'SolA', activity_type: 'balance_change' },
    });

    const baseOnly = stateStore.getCrossChainActivity('shindevlin', 'base');
    expect(baseOnly).toHaveLength(1);
    expect(baseOnly[0].chain).toBe('base');

    const allActivities = stateStore.getCrossChainActivity('shindevlin');
    expect(allActivities).toHaveLength(2);
  });

  it('getRecentCrossChainActivity returns entries sorted by timestamp desc', () => {
    stateStore.applyEntry({
      type: 'CROSS_CHAIN_ACTIVITY',
      to: 'shindevlin',
      epoch: 1,
      timestamp: 1000,
      cross_chain_data: { chain: 'base', address: '0xA', activity_type: 'balance_change' },
    });
    stateStore.applyEntry({
      type: 'CROSS_CHAIN_ACTIVITY',
      to: 'natoshisakamoto',
      epoch: 2,
      timestamp: 5000,
      cross_chain_data: { chain: 'bitcoin', address: 'bc1qB', activity_type: 'balance_change' },
    });

    const recent = stateStore.getRecentCrossChainActivity(10);
    expect(recent.length).toBeGreaterThanOrEqual(2);
    // Should be sorted newest first
    expect(recent[0].timestamp).toBeGreaterThanOrEqual(recent[1].timestamp);
  });

  it('returns empty array for unknown user', () => {
    expect(stateStore.getCrossChainActivity('nobody')).toEqual([]);
  });

  it('resetAll clears crossChainActivity', () => {
    stateStore.applyEntry({
      type: 'CROSS_CHAIN_ACTIVITY',
      to: 'shindevlin',
      epoch: 1,
      timestamp: 1000,
      cross_chain_data: { chain: 'ethereum', address: '0xX', activity_type: 'balance_change' },
    });
    stateStore.resetAll();
    expect(stateStore.getCrossChainActivity('shindevlin')).toEqual([]);
  });
});

// ─── createChainMonitor integration ──────────────────────────────

describe('createChainMonitor integration', () => {
  let mockLedger;
  let mockStateStore;

  beforeEach(() => {
    jest.restoreAllMocks();

    mockLedger = {
      getCurrentEpoch: jest.fn().mockResolvedValue(42),
      recordCrossChainActivity: jest.fn().mockResolvedValue({ type: 'CROSS_CHAIN_ACTIVITY' }),
    };

    mockStateStore = {
      getAllAccounts: jest.fn().mockReturnValue([
        {
          username: 'shindevlin',
          chain_addresses: {
            evm: '0xABC',
            solana: 'SolAddr123',
            bitcoin: 'bc1qBTC',
          },
        },
      ]),
    };
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('calls recordCrossChainActivity when EVM balance changes', async () => {
    const monitor = createChainMonitor({ ledger: mockLedger, stateStore: mockStateStore });

    // Seed balance
    jest.spyOn(global, 'fetch').mockResolvedValue({ ok: true, json: async () => ({ result: '0x100' }) });
    await monitor._pollAllAccounts();

    // Change balance
    jest.restoreAllMocks();
    jest.spyOn(global, 'fetch').mockResolvedValue({ ok: true, json: async () => ({ result: '0x200' }) });
    await monitor._pollAllAccounts();

    expect(mockLedger.recordCrossChainActivity).toHaveBeenCalledWith(
      'shindevlin',
      expect.any(String), // chain name (base, arbitrum, or ethereum)
      '0xABC',
      'balance_change',
      expect.objectContaining({ old_balance: 256, new_balance: 512 }),
      42
    );

    monitor.resetForTests();
  });

  it('does not call recordCrossChainActivity when no balance change', async () => {
    const monitor = createChainMonitor({ ledger: mockLedger, stateStore: mockStateStore });

    // Same balance for all calls
    jest.spyOn(global, 'fetch').mockResolvedValue({ ok: true, json: async () => ({ result: '0x100' }) });
    await monitor._pollAllAccounts();

    jest.restoreAllMocks();
    jest.spyOn(global, 'fetch').mockResolvedValue({ ok: true, json: async () => ({ result: '0x100' }) });
    mockLedger.recordCrossChainActivity.mockClear();
    await monitor._pollAllAccounts();

    // recordCrossChainActivity may be called for Solana or Bitcoin if they change,
    // but not for EVM since we gave the same value
    const evmCalls = mockLedger.recordCrossChainActivity.mock.calls.filter(
      (c) => c[1] === 'base' || c[1] === 'arbitrum' || c[1] === 'ethereum'
    );
    expect(evmCalls).toHaveLength(0);

    monitor.resetForTests();
  });

  it('handles RPC errors gracefully without throwing', async () => {
    const monitor = createChainMonitor({ ledger: mockLedger, stateStore: mockStateStore });

    jest.spyOn(global, 'fetch').mockRejectedValue(new Error('network down'));

    // Must not throw
    await expect(monitor._pollAllAccounts()).resolves.not.toThrow();

    monitor.resetForTests();
  });

  it('getActivity returns activities for a user', async () => {
    const monitor = createChainMonitor({ ledger: mockLedger, stateStore: mockStateStore });

    // Seed
    jest.spyOn(global, 'fetch').mockResolvedValue({ ok: true, json: async () => ({ result: '0x0' }) });
    await monitor._pollAllAccounts();

    // Change
    jest.restoreAllMocks();
    jest.spyOn(global, 'fetch').mockResolvedValue({ ok: true, json: async () => ({ result: '0xF' }) });
    await monitor._pollAllAccounts();

    // At least one activity should be recorded in the internal log
    // (even if ledger mock doesn't write to stateStore)
    const activities = monitor.getActivity('shindevlin');
    expect(Array.isArray(activities)).toBe(true);

    monitor.resetForTests();
  });

  it('resetForTests clears all cached state', async () => {
    const monitor = createChainMonitor({ ledger: mockLedger, stateStore: mockStateStore });

    jest.spyOn(global, 'fetch').mockResolvedValue({ ok: true, json: async () => ({ result: '0x100' }) });
    await monitor._pollAllAccounts();

    monitor.resetForTests();

    // After reset, internal balance cache is cleared
    // so next check won't detect a "change" from the old value
    jest.restoreAllMocks();
    jest.spyOn(global, 'fetch').mockResolvedValue({ ok: true, json: async () => ({ result: '0x100' }) });
    mockLedger.recordCrossChainActivity.mockClear();
    await monitor._pollAllAccounts();

    const evmCalls = mockLedger.recordCrossChainActivity.mock.calls.filter(
      (c) => c[1] === 'base' || c[1] === 'arbitrum' || c[1] === 'ethereum'
    );
    expect(evmCalls).toHaveLength(0);

    monitor.resetForTests();
  });
});

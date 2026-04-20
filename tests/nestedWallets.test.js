"use strict";

/**
 * Nested Wallets — v3.3
 * Shin Devlin
 *
 * Tests for the parent/child wallet namespace protocol.
 */

const stateStore = require('../src/chain/stateStore');

beforeEach(() => {
  stateStore.resetAll();
  // Create accounts needed for tests
  stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'shindevlin', epoch: 0, account_data: { public_keys: { posting: '03ead081' }, chain_addresses: {} } });
  stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'josh', epoch: 0, account_data: { public_keys: {}, chain_addresses: {} } });
  stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'btcpc', epoch: 0, account_data: { public_keys: { posting: '03ead081' }, chain_addresses: {} } });
});

// ──────────────────────────────────────────────────────────────────────────────
// WALLET_CREATE_CHILD entry applied via applyEntry
// ──────────────────────────────────────────────────────────────────────────────

describe('WALLET_CREATE_CHILD applyEntry', () => {
  test('creates child account with correct parent link', () => {
    stateStore.applyEntry({
      type: 'WALLET_CREATE_CHILD',
      from: 'shindevlin',
      to: 'shindevlin/vault',
      epoch: 1,
      wallet_data: { parent: 'shindevlin', child_name: 'vault' },
    });
    expect(stateStore.hasAccount('shindevlin/vault')).toBe(true);
    expect(stateStore.getParent('shindevlin/vault')).toBe('shindevlin');
  });

  test('registers child in getChildren list', () => {
    stateStore.applyEntry({
      type: 'WALLET_CREATE_CHILD',
      from: 'shindevlin',
      to: 'shindevlin/vault',
      epoch: 1,
      wallet_data: { parent: 'shindevlin', child_name: 'vault' },
    });
    const children = stateStore.getChildren('shindevlin');
    expect(children).toContain('shindevlin/vault');
  });

  test('multiple children under same parent', () => {
    stateStore.applyEntry({
      type: 'WALLET_CREATE_CHILD',
      from: 'shindevlin',
      to: 'shindevlin/vault',
      epoch: 1,
      wallet_data: { parent: 'shindevlin', child_name: 'vault' },
    });
    stateStore.applyEntry({
      type: 'WALLET_CREATE_CHILD',
      from: 'shindevlin',
      to: 'shindevlin/rewards',
      epoch: 1,
      wallet_data: { parent: 'shindevlin', child_name: 'rewards' },
    });
    const children = stateStore.getChildren('shindevlin');
    expect(children).toHaveLength(2);
    expect(children).toContain('shindevlin/vault');
    expect(children).toContain('shindevlin/rewards');
  });

  test('idempotent: second WALLET_CREATE_CHILD for same name is ignored', () => {
    const entry = {
      type: 'WALLET_CREATE_CHILD',
      from: 'shindevlin',
      to: 'shindevlin/vault',
      epoch: 1,
      timestamp: 100,
      wallet_data: { parent: 'shindevlin', child_name: 'vault' },
    };
    stateStore.applyEntry(entry);
    stateStore.applyEntry({ ...entry, timestamp: 200 });
    expect(stateStore.getChildren('shindevlin')).toHaveLength(1);
  });

  test('rejects child_name containing "/"', () => {
    stateStore.applyEntry({
      type: 'WALLET_CREATE_CHILD',
      from: 'shindevlin',
      to: 'shindevlin/a/b',
      epoch: 1,
      wallet_data: { parent: 'shindevlin', child_name: 'a/b' },
    });
    expect(stateStore.hasAccount('shindevlin/a/b')).toBe(false);
  });

  test('unknown parent still creates child (applyEntry is permissive)', () => {
    // applyEntry does not check parent existence — that's done at recordX level
    stateStore.applyEntry({
      type: 'WALLET_CREATE_CHILD',
      from: 'nobody',
      to: 'nobody/child',
      epoch: 1,
      wallet_data: { parent: 'nobody', child_name: 'child' },
    });
    expect(stateStore.hasAccount('nobody/child')).toBe(true);
    expect(stateStore.getParent('nobody/child')).toBe('nobody');
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// isAuthorizedBy
// ──────────────────────────────────────────────────────────────────────────────

describe('isAuthorizedBy', () => {
  beforeEach(() => {
    stateStore.applyEntry({
      type: 'WALLET_CREATE_CHILD',
      from: 'shindevlin',
      to: 'shindevlin/vault',
      epoch: 1,
      wallet_data: { parent: 'shindevlin', child_name: 'vault' },
    });
  });

  test('account authorizes itself', () => {
    expect(stateStore.isAuthorizedBy('shindevlin', 'shindevlin')).toBe(true);
  });

  test('parent authorizes child', () => {
    expect(stateStore.isAuthorizedBy('shindevlin/vault', 'shindevlin')).toBe(true);
  });

  test('unrelated account is not authorized', () => {
    expect(stateStore.isAuthorizedBy('shindevlin/vault', 'josh')).toBe(false);
  });

  test('child does not authorize parent', () => {
    expect(stateStore.isAuthorizedBy('shindevlin', 'shindevlin/vault')).toBe(false);
  });

  test('null inputs return false', () => {
    expect(stateStore.isAuthorizedBy(null, 'shindevlin')).toBe(false);
    expect(stateStore.isAuthorizedBy('shindevlin', null)).toBe(false);
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// getParent / getChildren
// ──────────────────────────────────────────────────────────────────────────────

describe('getParent / getChildren', () => {
  test('getParent returns null for top-level account', () => {
    expect(stateStore.getParent('shindevlin')).toBeNull();
  });

  test('getChildren returns empty array for childless account', () => {
    expect(stateStore.getChildren('josh')).toEqual([]);
  });

  test('getChildren for non-existent account returns empty array', () => {
    expect(stateStore.getChildren('nobody')).toEqual([]);
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// btcpc root + btcpc/treasury wiring (genesis simulation)
// ──────────────────────────────────────────────────────────────────────────────

describe('btcpc root namespace (genesis wiring)', () => {
  test('btcpc/treasury created as child of btcpc', () => {
    // Simulate genesis entries
    stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'btcpc_treasury', epoch: 0, account_data: { public_keys: {}, chain_addresses: {} } });
    stateStore.applyEntry({
      type: 'WALLET_CREATE_CHILD',
      from: 'btcpc',
      to: 'btcpc/treasury',
      epoch: 0,
      wallet_data: { parent: 'btcpc', child_name: 'treasury' },
    });
    expect(stateStore.hasAccount('btcpc/treasury')).toBe(true);
    expect(stateStore.getParent('btcpc/treasury')).toBe('btcpc');
    expect(stateStore.getChildren('btcpc')).toContain('btcpc/treasury');
  });

  test('btcpc is authorized to act for btcpc/treasury', () => {
    stateStore.applyEntry({
      type: 'WALLET_CREATE_CHILD',
      from: 'btcpc',
      to: 'btcpc/treasury',
      epoch: 0,
      wallet_data: { parent: 'btcpc', child_name: 'treasury' },
    });
    expect(stateStore.isAuthorizedBy('btcpc/treasury', 'btcpc')).toBe(true);
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// btcpc_recycle keyless enforcement
// ──────────────────────────────────────────────────────────────────────────────

describe('btcpc_recycle key guard', () => {
  test('btcpc_recycle account exists after resetAll', () => {
    expect(stateStore.hasAccount('btcpc_recycle')).toBe(true);
  });

  test('ACCOUNT_CREATE with keys for btcpc_recycle is rejected', () => {
    stateStore.applyEntry({
      type: 'ACCOUNT_CREATE',
      to: 'btcpc_recycle',
      epoch: 0,
      account_data: { public_keys: { posting: 'deadbeef' }, chain_addresses: {} },
    });
    const acc = stateStore.getAccount('btcpc_recycle');
    // Keys must remain empty
    expect(Object.keys(acc.public_keys || {})).toHaveLength(0);
  });

  test('KEY_ROTATE targeting btcpc_recycle is rejected', () => {
    stateStore.applyEntry({
      type: 'KEY_ROTATE',
      to: 'btcpc_recycle',
      epoch: 1,
      key_rotate_data: { new_public_keys: { posting: 'deadbeef' } },
    });
    const acc = stateStore.getAccount('btcpc_recycle');
    expect(Object.keys(acc.public_keys || {})).toHaveLength(0);
  });

  test('btcpc_recycle can receive balance (credit)', () => {
    stateStore.applyEntry({
      type: 'MINING_REWARD',
      to: 'btcpc_recycle',
      amount: 100,
      epoch: 1,
    });
    expect(stateStore.getBalance('btcpc_recycle', 'BTCPC')).toBe(100);
  });
});

// ──────────────────────────────────────────────────────────────────────────────
// resetAll clears nested wallet state
// ──────────────────────────────────────────────────────────────────────────────

describe('resetAll clears nested wallet state', () => {
  test('childrenMap and parentMap cleared on resetAll', () => {
    stateStore.applyEntry({
      type: 'WALLET_CREATE_CHILD',
      from: 'shindevlin',
      to: 'shindevlin/vault',
      epoch: 1,
      wallet_data: { parent: 'shindevlin', child_name: 'vault' },
    });
    stateStore.resetAll();
    expect(stateStore.getChildren('shindevlin')).toEqual([]);
    expect(stateStore.getParent('shindevlin/vault')).toBeNull();
  });
});

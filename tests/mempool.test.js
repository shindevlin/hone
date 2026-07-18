const mempool = require('../src/p2p/mempool');

describe('mempool', () => {
  beforeEach(() => {
    mempool.clear();
  });

  test('submit accepts a valid transaction and tracks pending debit', () => {
    const result = mempool.submit({
      type: 'TRANSFER',
      from: 'alice',
      to: 'bob',
      amount: 3,
      token: 'HONE',
      nonce: 1,
      timestamp: 100
    });

    expect(result.accepted).toBe(true);
    expect(result.txHash).toMatch(/^[0-9a-f]{64}$/);
    expect(mempool.size()).toBe(1);
    expect(mempool.getPendingDebit('alice')).toBe(3);
  });

  test('duplicate transaction is rejected by hash', () => {
    const tx = {
      type: 'TRANSFER',
      from: 'alice',
      to: 'bob',
      amount: 3,
      nonce: 1,
      timestamp: 100
    };

    expect(mempool.submit({ ...tx }).accepted).toBe(true);
    const duplicate = mempool.submit({ ...tx });

    expect(duplicate.accepted).toBe(false);
    expect(duplicate.reason).toBe('duplicate');
    expect(mempool.size()).toBe(1);
  });

  test('nonce reuse is rejected even when transaction hash differs', () => {
    expect(mempool.submit({
      type: 'TRANSFER',
      from: 'alice',
      to: 'bob',
      amount: 3,
      nonce: 7,
      timestamp: 100
    }).accepted).toBe(true);

    const replay = mempool.submit({
      type: 'TRANSFER',
      from: 'alice',
      to: 'carol',
      amount: 2,
      nonce: 7,
      timestamp: 101
    });

    expect(replay.accepted).toBe(false);
    expect(replay.reason).toBe('nonce reuse');
    expect(mempool.getPendingDebit('alice')).toBe(3);
  });

  test('removeTransaction clears pending debit and nonce tracking', () => {
    const accepted = mempool.submit({
      type: 'TRANSFER',
      from: 'alice',
      to: 'bob',
      amount: 4,
      nonce: 9,
      timestamp: 100
    });

    expect(mempool.removeTransaction(accepted.txHash)).toBe(true);
    expect(mempool.getPendingDebit('alice')).toBe(0);

    const reuseAfterRemoval = mempool.submit({
      type: 'TRANSFER',
      from: 'alice',
      to: 'carol',
      amount: 1,
      nonce: 9,
      timestamp: 101
    });
    expect(reuseAfterRemoval.accepted).toBe(true);
  });
});

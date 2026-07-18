"use strict";

/**
 * HONE-FS payout tests — v2.11.1
 *
 * Tests the pro-rata payout distribution logic in isolation (computePayouts)
 * and with real ledger + stateStore wiring (settlePayouts).
 */

const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
}));

const payouts = require('../src/services/blobPayouts');
const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');
const epochBandwidth = require('../src/services/epochBandwidth');

const CID_A = 'a'.repeat(64);
const CID_B = 'b'.repeat(64);

describe('blobPayouts (v2.11.1)', () => {
  beforeEach(() => {
    stateStore.resetAll();
    epochBandwidth.resetAll();
    ledger.flushPendingEntries();
    mockMempoolSubmit.mockReset();
    mockMempoolSubmit.mockReturnValue({ accepted: true });
    // Seed test accounts with enough EB to cover all test operations
    ['shindevlin', 'alice', 'bob', 'carol'].forEach(account => epochBandwidth.seedForTest(account));
  });

  describe('computePayouts (pure)', () => {
    test('null blob commit returns null', () => {
      expect(payouts.computePayouts(null)).toBeNull();
    });

    test('90/9/1 split of the pool', () => {
      const result = payouts.computePayouts({
        cid: CID_A,
        payment_hone: 100,
        bytes_served_by_host: { alice: 1000 },
      });
      expect(result.total_pool).toBe(100);
      expect(result.host_pool).toBe(90);
      expect(result.recycle_pool).toBe(9);
      expect(result.reputation_pool).toBe(1);
    });

    test('single host gets 100% of host pool', () => {
      const result = payouts.computePayouts({
        cid: CID_A,
        payment_hone: 100,
        bytes_served_by_host: { alice: 1000 },
      });
      expect(result.host_payouts.length).toBe(1);
      expect(result.host_payouts[0].host).toBe('alice');
      expect(result.host_payouts[0].amount_hone).toBe(90);
    });

    test('two hosts split pro-rata by bytes served', () => {
      const result = payouts.computePayouts({
        cid: CID_A,
        payment_hone: 1000,
        bytes_served_by_host: { alice: 300, bob: 700 },
      });
      // host_pool = 900
      // alice = 900 * (300/1000) = 270
      // bob   = 900 * (700/1000) = 630
      const byHost = {};
      result.host_payouts.forEach((p) => { byHost[p.host] = p.amount_hone; });
      expect(byHost.alice).toBeCloseTo(270, 5);
      expect(byHost.bob).toBeCloseTo(630, 5);
    });

    test('three hosts split equally when bytes equal', () => {
      const result = payouts.computePayouts({
        cid: CID_A,
        payment_hone: 300,
        bytes_served_by_host: { alice: 100, bob: 100, carol: 100 },
      });
      // host_pool = 270, each = 90
      result.host_payouts.forEach((p) => {
        expect(p.amount_hone).toBeCloseTo(90, 5);
      });
    });

    test('zero bytes served → zero host payouts (pool preserved)', () => {
      const result = payouts.computePayouts({
        cid: CID_A,
        payment_hone: 100,
        bytes_served_by_host: {},
      });
      expect(result.total_bytes_served).toBe(0);
      expect(result.host_payouts).toEqual([]);
      // Host pool calculation still computed, just not distributed
      expect(result.host_pool).toBe(90);
    });

    test('zero pool → zero payouts', () => {
      const result = payouts.computePayouts({
        cid: CID_A,
        payment_hone: 0,
        bytes_served_by_host: { alice: 1000 },
      });
      expect(result.host_pool).toBe(0);
      expect(result.recycle_pool).toBe(0);
      expect(result.reputation_pool).toBe(0);
    });

    test('host with zero bytes in non-empty map gets zero', () => {
      const result = payouts.computePayouts({
        cid: CID_A,
        payment_hone: 100,
        bytes_served_by_host: { alice: 0, bob: 1000 },
      });
      const byHost = {};
      result.host_payouts.forEach((p) => { byHost[p.host] = p.amount_hone; });
      expect(byHost.alice).toBe(0);
      expect(byHost.bob).toBe(90);
    });
  });

  describe('settlePayouts (end-to-end with ledger)', () => {
    beforeEach(() => {
      // Seed uploader with HONE so recordTransfer succeeds
      stateStore.applyEntry({
        type: 'FAUCET',
        from: 'hone_genesis',
        to: 'shindevlin',
        token: 'HONE',
        amount: 10000,
        epoch: 0,
        timestamp: 1,
      });
    });

    test('settles single-host pool end-to-end', async () => {
      // Commit CID with payment
      await ledger.recordBlobStoreCommit(
        'shindevlin',
        CID_A,
        1000,
        ['alice'],
        100,
        100, // 100 HONE payment pool
        1
      );
      // Alice serves some bytes
      await ledger.recordBlobServeProof('alice', CID_A, 5000, 10, null, 2);

      const blob = stateStore.getBlobCommit(CID_A);
      const result = await payouts.settlePayouts(blob, {
        uploader: 'shindevlin',
        epoch: 3,
      });

      expect(result.transfers.length).toBeGreaterThan(0);
      // Alice should receive 90 HONE (the full host pool)
      expect(stateStore.getBalance('alice', 'HONE')).toBeCloseTo(90, 5);
      // Recycle gets 9
      expect(stateStore.getBalance('hone_recycle', 'HONE')).toBeCloseTo(9, 5);
      // Reputation pool gets 1
      expect(stateStore.getBalance('hone_reputation_pool', 'HONE')).toBeCloseTo(1, 5);
      // Uploader paid 100 total
      expect(stateStore.getBalance('shindevlin', 'HONE')).toBeCloseTo(10000 - 100, 5);
    });

    test('settles three-host pool pro-rata', async () => {
      await ledger.recordBlobStoreCommit(
        'shindevlin',
        CID_A,
        1000,
        ['alice', 'bob', 'carol'],
        100,
        1000, // 1000 HONE payment pool
        1
      );
      await ledger.recordBlobServeProof('alice', CID_A, 100, 1, null, 2); // 10%
      await ledger.recordBlobServeProof('bob', CID_A, 300, 3, null, 2);   // 30%
      await ledger.recordBlobServeProof('carol', CID_A, 600, 6, null, 2); // 60%

      const blob = stateStore.getBlobCommit(CID_A);
      await payouts.settlePayouts(blob, { uploader: 'shindevlin', epoch: 3 });

      // host_pool = 900
      // alice = 90, bob = 270, carol = 540
      expect(stateStore.getBalance('alice', 'HONE')).toBeCloseTo(90, 5);
      expect(stateStore.getBalance('bob', 'HONE')).toBeCloseTo(270, 5);
      expect(stateStore.getBalance('carol', 'HONE')).toBeCloseTo(540, 5);
      expect(stateStore.getBalance('hone_recycle', 'HONE')).toBeCloseTo(90, 5);
      expect(stateStore.getBalance('hone_reputation_pool', 'HONE')).toBeCloseTo(10, 5);
    });

    test('no-bytes-served returns empty transfers, no debits', async () => {
      await ledger.recordBlobStoreCommit(
        'shindevlin',
        CID_A,
        1000,
        ['alice'],
        100,
        100,
        1
      );
      const blob = stateStore.getBlobCommit(CID_A);
      const result = await payouts.settlePayouts(blob, {
        uploader: 'shindevlin',
        epoch: 2,
      });
      expect(result.transfers).toEqual([]);
      expect(stateStore.getBalance('alice', 'HONE')).toBe(0);
      expect(stateStore.getBalance('shindevlin', 'HONE')).toBe(10000);
    });

    test('zero pool returns empty transfers', async () => {
      await ledger.recordBlobStoreCommit(
        'shindevlin',
        CID_A,
        1000,
        ['alice'],
        100,
        0, // zero payment
        1
      );
      await ledger.recordBlobServeProof('alice', CID_A, 5000, 10, null, 2);
      const blob = stateStore.getBlobCommit(CID_A);
      const result = await payouts.settlePayouts(blob, {
        uploader: 'shindevlin',
        epoch: 3,
      });
      expect(result.transfers).toEqual([]);
    });

    test('missing uploader with no blob fallback throws', async () => {
      // Synthetic blob commit with served bytes but no uploader recorded
      const syntheticBlob = {
        cid: CID_A,
        size: 1000,
        hosts: ['alice'],
        uploader: null,
        payment_hone: 100,
        bytes_served_by_host: { alice: 5000 },
      };
      await expect(
        payouts.settlePayouts(syntheticBlob, { uploader: null, epoch: 3 })
      ).rejects.toThrow('uploader required');
    });

    test('falls back to blob.uploader when options.uploader is missing', async () => {
      await ledger.recordBlobStoreCommit('shindevlin', CID_A, 1000, ['alice'], 100, 100, 1);
      await ledger.recordBlobServeProof('alice', CID_A, 5000, 10, null, 2);
      const blob = stateStore.getBlobCommit(CID_A);
      // Pass no uploader option — should use blob.uploader
      const result = await payouts.settlePayouts(blob, { epoch: 3 });
      expect(result.transfers.length).toBe(3);
      expect(stateStore.getBalance('alice', 'HONE')).toBeCloseTo(90, 5);
    });
  });
});

"use strict";

/**
 * HONE-FS bandwidth accumulator tests — v2.11.1
 *
 * Tests the in-memory byte counter and its flush-to-chain integration.
 * Uses dependency injection to avoid pulling in the full ledger/stateStore
 * graph where not needed, and uses the real modules for the end-to-end
 * flush tests.
 */

const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
}));

const bw = require('../src/services/blobBandwidth');
const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');
const epochBandwidth = require('../src/services/epochBandwidth');

const CID_A = 'a'.repeat(64);
const CID_B = 'b'.repeat(64);
const CID_C = 'c'.repeat(64);

describe('blobBandwidth (v2.11.1)', () => {
  beforeEach(() => {
    bw.resetForTests();
    stateStore.resetAll();
    epochBandwidth.resetAll();
    ledger.flushPendingEntries();
    mockMempoolSubmit.mockReset();
    mockMempoolSubmit.mockReturnValue({ accepted: true });
    ['alice', 'bob', 'carol'].forEach(a => epochBandwidth.seedForTest(a));
  });

  describe('accumulate / getPending', () => {
    test('starts with zero for a new CID', () => {
      expect(bw.getPending(CID_A)).toEqual({ bytes: 0, requests: 0 });
    });

    test('accumulate adds bytes and increments request count', () => {
      bw.accumulate(CID_A, 100);
      expect(bw.getPending(CID_A)).toEqual({ bytes: 100, requests: 1 });
      bw.accumulate(CID_A, 250);
      expect(bw.getPending(CID_A)).toEqual({ bytes: 350, requests: 2 });
    });

    test('accumulate ignores zero/negative bytes', () => {
      bw.accumulate(CID_A, 0);
      bw.accumulate(CID_A, -100);
      expect(bw.getPending(CID_A)).toEqual({ bytes: 0, requests: 0 });
    });

    test('accumulate ignores invalid CIDs', () => {
      bw.accumulate('', 100);
      bw.accumulate(null, 100);
      bw.accumulate(undefined, 100);
      expect(bw.pendingCidCount()).toBe(0);
    });

    test('multiple CIDs accumulate independently', () => {
      bw.accumulate(CID_A, 100);
      bw.accumulate(CID_B, 200);
      bw.accumulate(CID_A, 50);
      expect(bw.getPending(CID_A).bytes).toBe(150);
      expect(bw.getPending(CID_B).bytes).toBe(200);
      expect(bw.pendingCidCount()).toBe(2);
    });

    test('getPending() with no arg returns all', () => {
      bw.accumulate(CID_A, 10);
      bw.accumulate(CID_B, 20);
      const all = bw.getPending();
      expect(Object.keys(all).sort()).toEqual([CID_A, CID_B].sort());
      expect(all[CID_A].bytes).toBe(10);
      expect(all[CID_B].bytes).toBe(20);
    });
  });

  describe('clearPending', () => {
    test('removes specific CID', () => {
      bw.accumulate(CID_A, 100);
      bw.accumulate(CID_B, 200);
      expect(bw.clearPending(CID_A)).toBe(true);
      expect(bw.getPending(CID_A).bytes).toBe(0);
      expect(bw.getPending(CID_B).bytes).toBe(200);
    });

    test('returns false for unknown CID', () => {
      expect(bw.clearPending(CID_A)).toBe(false);
    });
  });

  describe('flushServeProofs integration', () => {
    test('flushes pending bytes for committed hosts', async () => {
      // Commit CID_A with alice as host
      await ledger.recordBlobStoreCommit('alice', CID_A, 1000, ['alice'], 100, 0, 1);

      // Accumulate some bytes
      bw.accumulate(CID_A, 500);
      bw.accumulate(CID_A, 750);

      const result = await bw.flushServeProofs('alice');
      expect(result.flushed).toBe(1);
      expect(result.skipped).toBe(0);

      // Pending should be cleared
      expect(bw.getPending(CID_A).bytes).toBe(0);

      // Chain should reflect the serve proof
      const commit = stateStore.getBlobCommit(CID_A);
      expect(commit.bytes_served_total).toBe(1250);
      expect(commit.bytes_served_by_host.alice).toBe(1250);
    });

    test('skips CIDs with no commit', async () => {
      bw.accumulate(CID_A, 500);
      const result = await bw.flushServeProofs('alice');
      expect(result.flushed).toBe(0);
      expect(result.skipped).toBe(1);
      expect(result.details[0].status).toBe('no_commit');
      // Pending is retained (operator may commit later and re-flush)
      expect(bw.getPending(CID_A).bytes).toBe(500);
    });

    test('skips CIDs where host is not committed', async () => {
      await ledger.recordBlobStoreCommit('alice', CID_A, 1000, ['alice'], 100, 0, 1);
      bw.accumulate(CID_A, 500);
      // bob tries to flush but alice is the only committed host
      const result = await bw.flushServeProofs('bob');
      expect(result.flushed).toBe(0);
      expect(result.skipped).toBe(1);
      expect(result.details[0].status).toBe('not_committed_host');
      // bob's attempted flush leaves the counter intact
      expect(bw.getPending(CID_A).bytes).toBe(500);
    });

    test('flushes multiple CIDs in one call', async () => {
      await ledger.recordBlobStoreCommit('alice', CID_A, 100, ['alice'], 100, 0, 1);
      await ledger.recordBlobStoreCommit('alice', CID_B, 100, ['alice'], 100, 0, 1);
      await ledger.recordBlobStoreCommit('bob', CID_C, 100, ['bob'], 100, 0, 1);

      bw.accumulate(CID_A, 100);
      bw.accumulate(CID_B, 200);
      bw.accumulate(CID_C, 300);

      // alice flushes her two, skips bob's CID_C
      const result = await bw.flushServeProofs('alice');
      expect(result.flushed).toBe(2);
      expect(result.skipped).toBe(1);

      expect(stateStore.getBlobCommit(CID_A).bytes_served_total).toBe(100);
      expect(stateStore.getBlobCommit(CID_B).bytes_served_total).toBe(200);
      expect(stateStore.getBlobCommit(CID_C).bytes_served_total).toBe(0); // bob's, not flushed by alice
    });

    test('empty pending state returns zero flushed', async () => {
      const result = await bw.flushServeProofs('alice');
      expect(result.flushed).toBe(0);
      expect(result.skipped).toBe(0);
    });

    test('missing host returns zero', async () => {
      bw.accumulate(CID_A, 100);
      const result = await bw.flushServeProofs(null);
      expect(result.flushed).toBe(0);
      expect(result.skipped).toBe(0);
    });
  });
});

"use strict";

/**
 * HONE-FS serve proof tests — v2.11.1
 *
 * Exercises the BLOB_SERVE_PROOF ledger entry type and stateStore
 * dispatcher. Tests chain invariants:
 *   - commit required before serve proof
 *   - only committed hosts can serve
 *   - bytes accumulate per-host + total
 *   - multiple proofs in the same epoch don't dedupe
 */

const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
}));

const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');
const epochBandwidth = require('../src/services/epochBandwidth');

const TEST_CID = 'b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9';

describe('BLOB_SERVE_PROOF (v2.11.1)', () => {
  beforeEach(() => {
    stateStore.resetAll();
    epochBandwidth.resetAll();
    ledger.flushPendingEntries();
    mockMempoolSubmit.mockReset();
    mockMempoolSubmit.mockReturnValue({ accepted: true });
    ['alice', 'bob', 'carol', 'shindevlin'].forEach(a => epochBandwidth.seedForTest(a));
  });

  describe('recordBlobServeProof validation', () => {
    test('rejects missing host', async () => {
      await expect(
        ledger.recordBlobServeProof(null, TEST_CID, 1024, 1, null, 1)
      ).rejects.toThrow('host required');
    });

    test('rejects invalid CID format', async () => {
      await expect(
        ledger.recordBlobServeProof('alice', 'bad-cid', 1024, 1, null, 1)
      ).rejects.toThrow('64-char hex');
    });

    test('rejects negative bytes_served', async () => {
      await expect(
        ledger.recordBlobServeProof('alice', TEST_CID, -1, 1, null, 1)
      ).rejects.toThrow('non-negative');
    });

    test('rejects non-numeric bytes_served', async () => {
      await expect(
        ledger.recordBlobServeProof('alice', TEST_CID, 'many', 1, null, 1)
      ).rejects.toThrow('non-negative');
    });

    test('accepts zero bytes served (empty epoch report)', async () => {
      await ledger.recordBlobStoreCommit('alice', TEST_CID, 1000, ['alice'], 100, 0, 1);
      await ledger.recordBlobServeProof('alice', TEST_CID, 0, 0, null, 2);
      // No throw = success. Blob record should still show 0 served.
      const b = stateStore.getBlobCommit(TEST_CID);
      expect(b.bytes_served_total).toBe(0);
      expect(b.serve_proof_count).toBe(1);
    });
  });

  describe('chain invariants', () => {
    test('serve proof without a commit is silently dropped', async () => {
      await ledger.recordBlobServeProof('alice', TEST_CID, 1024, 1, null, 1);
      expect(stateStore.getBlobCommit(TEST_CID)).toBeNull();
    });

    test('serve proof from non-committed host is dropped', async () => {
      await ledger.recordBlobStoreCommit('alice', TEST_CID, 1000, ['alice'], 100, 0, 1);
      // mallory is NOT in the hosts list
      await ledger.recordBlobServeProof('mallory', TEST_CID, 1024, 1, null, 2);
      const b = stateStore.getBlobCommit(TEST_CID);
      expect(b.bytes_served_total).toBe(0);
      expect(b.bytes_served_by_host.mallory).toBeUndefined();
    });

    test('serve proof from committed host accumulates', async () => {
      await ledger.recordBlobStoreCommit('alice', TEST_CID, 1000, ['alice'], 100, 0, 1);
      await ledger.recordBlobServeProof('alice', TEST_CID, 1024, 1, null, 2);
      const b = stateStore.getBlobCommit(TEST_CID);
      expect(b.bytes_served_total).toBe(1024);
      expect(b.bytes_served_by_host.alice).toBe(1024);
      expect(b.serve_proof_count).toBe(1);
    });
  });

  describe('multi-host replication + aggregation', () => {
    test('multi-host commit accepts serve proofs from all hosts', async () => {
      await ledger.recordBlobStoreCommit(
        'alice',
        TEST_CID,
        1000,
        ['alice', 'bob', 'carol'],
        100,
        30,
        1
      );
      const b = stateStore.getBlobCommit(TEST_CID);
      expect(b.hosts.sort()).toEqual(['alice', 'bob', 'carol']);

      await ledger.recordBlobServeProof('alice', TEST_CID, 500, 1, null, 2);
      await ledger.recordBlobServeProof('bob', TEST_CID, 1500, 3, null, 2);
      await ledger.recordBlobServeProof('carol', TEST_CID, 250, 1, null, 2);

      const updated = stateStore.getBlobCommit(TEST_CID);
      expect(updated.bytes_served_total).toBe(2250);
      expect(updated.bytes_served_by_host.alice).toBe(500);
      expect(updated.bytes_served_by_host.bob).toBe(1500);
      expect(updated.bytes_served_by_host.carol).toBe(250);
      expect(updated.serve_proof_count).toBe(3);
    });

    test('multiple serve proofs from same host in same epoch accumulate', async () => {
      await ledger.recordBlobStoreCommit('alice', TEST_CID, 1000, ['alice'], 100, 0, 1);
      await ledger.recordBlobServeProof('alice', TEST_CID, 100, 1, 'root-1', 2);
      await ledger.recordBlobServeProof('alice', TEST_CID, 200, 2, 'root-2', 2);
      await ledger.recordBlobServeProof('alice', TEST_CID, 300, 3, 'root-3', 2);
      const b = stateStore.getBlobCommit(TEST_CID);
      expect(b.bytes_served_total).toBe(600);
      expect(b.bytes_served_by_host.alice).toBe(600);
      expect(b.serve_proof_count).toBe(3);
    });

    test('later commits can add new hosts and accept their proofs', async () => {
      // Initial commit with just alice
      await ledger.recordBlobStoreCommit('uploader', TEST_CID, 1000, ['alice'], 100, 0, 1);
      await ledger.recordBlobServeProof('bob', TEST_CID, 500, 1, null, 2);
      expect(stateStore.getBlobCommit(TEST_CID).bytes_served_total).toBe(0);

      // Extend with bob
      await ledger.recordBlobStoreCommit('uploader', TEST_CID, 1000, ['bob'], 100, 0, 3);
      expect(stateStore.getBlobCommit(TEST_CID).hosts.sort()).toEqual(['alice', 'bob']);

      // Now bob's proofs work
      await ledger.recordBlobServeProof('bob', TEST_CID, 800, 1, null, 4);
      const b = stateStore.getBlobCommit(TEST_CID);
      expect(b.bytes_served_total).toBe(800);
      expect(b.bytes_served_by_host.bob).toBe(800);
    });
  });

  describe('aggregation queries', () => {
    test('getBlobCommitsByHost returns blobs where host is committed', async () => {
      const cid1 = 'a'.repeat(64);
      const cid2 = 'b'.repeat(64);
      const cid3 = 'c'.repeat(64);
      await ledger.recordBlobStoreCommit('alice', cid1, 100, ['alice', 'bob'], 100, 0, 1);
      await ledger.recordBlobStoreCommit('alice', cid2, 100, ['bob', 'carol'], 100, 0, 1);
      await ledger.recordBlobStoreCommit('alice', cid3, 100, ['alice'], 100, 0, 1);

      const bobCommits = stateStore.getBlobCommitsByHost('bob');
      expect(bobCommits.length).toBe(2);
      const cids = bobCommits.map((b) => b.cid).sort();
      expect(cids).toEqual([cid1, cid2]);
    });

    test('getBlobCommitsByUploader returns blobs uploaded by the account', async () => {
      const cid1 = 'a'.repeat(64);
      const cid2 = 'b'.repeat(64);
      await ledger.recordBlobStoreCommit('alice', cid1, 100, ['alice'], 100, 0, 1);
      await ledger.recordBlobStoreCommit('bob', cid2, 100, ['bob'], 100, 0, 1);

      const aliceBlobs = stateStore.getBlobCommitsByUploader('alice');
      expect(aliceBlobs.length).toBe(1);
      expect(aliceBlobs[0].cid).toBe(cid1);
    });
  });
});

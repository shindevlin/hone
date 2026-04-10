"use strict";

/**
 * STORAGE_HEARTBEAT tests — v2.11.2
 *
 * Tests the host liveness signal + uptime factor calculation.
 * Home-user durability primitive — hosts with flaky ISPs can still
 * earn payout shares as long as their heartbeat ratio is non-zero.
 */

const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
}));

const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');

const CID_A = 'a'.repeat(64);
const CID_B = 'b'.repeat(64);
const CID_C = 'c'.repeat(64);

describe('STORAGE_HEARTBEAT (v2.11.2)', () => {
  beforeEach(() => {
    stateStore.resetAll();
    ledger.flushPendingEntries();
    mockMempoolSubmit.mockReset();
    mockMempoolSubmit.mockReturnValue({ accepted: true });
  });

  describe('recordStorageHeartbeat validation', () => {
    test('requires host', async () => {
      await expect(
        ledger.recordStorageHeartbeat(null, [CID_A], 100, 1)
      ).rejects.toThrow('host required');
    });

    test('requires cids array', async () => {
      await expect(
        ledger.recordStorageHeartbeat('alice', null, 100, 1)
      ).rejects.toThrow('cids must be an array');
    });

    test('filters invalid CID formats', async () => {
      await ledger.recordStorageHeartbeat(
        'alice',
        [CID_A, 'bad-cid', CID_B, 'ABCDEF'.repeat(10) + 'xx', CID_C],
        100,
        1
      );
      const hb = stateStore.getStorageHeartbeat('alice');
      expect(hb.heartbeats[0].cids).toEqual([CID_A, CID_B, CID_C]);
    });

    test('empty cids array is valid (host with nothing committed)', async () => {
      await ledger.recordStorageHeartbeat('alice', [], 0, 1);
      const hb = stateStore.getStorageHeartbeat('alice');
      expect(hb.total_heartbeats).toBe(1);
      expect(hb.heartbeats[0].cids).toEqual([]);
    });
  });

  describe('heartbeat recording', () => {
    test('first heartbeat creates host record', async () => {
      await ledger.recordStorageHeartbeat('alice', [CID_A], 50, 1);
      const hb = stateStore.getStorageHeartbeat('alice');
      expect(hb).toBeTruthy();
      expect(hb.host).toBe('alice');
      expect(hb.total_heartbeats).toBe(1);
      expect(hb.last_heartbeat_epoch).toBe(1);
      expect(hb.first_heartbeat_epoch).toBe(1);
    });

    test('subsequent heartbeats accumulate', async () => {
      await ledger.recordStorageHeartbeat('alice', [CID_A], 50, 1);
      await ledger.recordStorageHeartbeat('alice', [CID_A, CID_B], 100, 10);
      await ledger.recordStorageHeartbeat('alice', [CID_A, CID_B, CID_C], 150, 20);
      const hb = stateStore.getStorageHeartbeat('alice');
      expect(hb.total_heartbeats).toBe(3);
      expect(hb.last_heartbeat_epoch).toBe(20);
      expect(hb.first_heartbeat_epoch).toBe(1);
      expect(hb.heartbeats.length).toBe(3);
    });

    test('multiple hosts tracked independently', async () => {
      await ledger.recordStorageHeartbeat('alice', [CID_A], 50, 1);
      await ledger.recordStorageHeartbeat('bob', [CID_B], 75, 2);
      expect(stateStore.getStorageHeartbeat('alice').total_heartbeats).toBe(1);
      expect(stateStore.getStorageHeartbeat('bob').total_heartbeats).toBe(1);
      expect(stateStore.getAllStorageHosts().length).toBe(2);
    });

    test('returns null for unknown host', () => {
      expect(stateStore.getStorageHeartbeat('nobody')).toBeNull();
    });
  });

  describe('getStorageUptimeFactor', () => {
    test('returns 0 for host with no record', () => {
      const factor = stateStore.getStorageUptimeFactor('nobody', 100, 100, 5);
      expect(factor).toBe(0);
    });

    test('perfect uptime returns 1.0', async () => {
      // Heartbeat at every expected interval (every 5 epochs) over a 100-epoch window
      for (let e = 1; e <= 100; e += 5) {
        await ledger.recordStorageHeartbeat('alice', [CID_A], 50, e);
      }
      const factor = stateStore.getStorageUptimeFactor('alice', 100, 100, 5);
      expect(factor).toBe(1.0);
    });

    test('50% uptime returns ~0.5', async () => {
      // Heartbeat at every 10 epochs instead of every 5 → half the expected rate
      for (let e = 1; e <= 100; e += 10) {
        await ledger.recordStorageHeartbeat('alice', [CID_A], 50, e);
      }
      const factor = stateStore.getStorageUptimeFactor('alice', 100, 100, 5);
      expect(factor).toBeGreaterThan(0.4);
      expect(factor).toBeLessThan(0.6);
    });

    test('zero heartbeats in window returns 0', async () => {
      // Heartbeat way outside the window
      await ledger.recordStorageHeartbeat('alice', [CID_A], 50, 1);
      const factor = stateStore.getStorageUptimeFactor('alice', 500, 100, 5);
      expect(factor).toBe(0);
    });

    test('excess heartbeats are capped at 1.0', async () => {
      // Heartbeat every epoch (5x the expected rate)
      for (let e = 1; e <= 100; e++) {
        await ledger.recordStorageHeartbeat('alice', [CID_A], 50, e);
      }
      const factor = stateStore.getStorageUptimeFactor('alice', 100, 100, 5);
      expect(factor).toBe(1.0);
    });

    test('home-user-friendly: a few missed beats still earns partial credit', async () => {
      // Heartbeat most intervals but miss a few (flaky ISP)
      const beats = [1, 6, 11, 16, 26, 36, 46, 56, 66, 76, 86, 96]; // missed 21, 31
      for (const e of beats) {
        await ledger.recordStorageHeartbeat('alice', [CID_A], 50, e);
      }
      const factor = stateStore.getStorageUptimeFactor('alice', 100, 100, 5);
      // Expected: 20 heartbeats in 100-epoch window (one every 5 epochs)
      // Got: 12. Ratio = 0.6
      expect(factor).toBeGreaterThan(0.5);
      expect(factor).toBeLessThan(0.7);
    });
  });

  describe('getActiveStorageHosts', () => {
    test('empty when no hosts', () => {
      expect(stateStore.getActiveStorageHosts(100, 50)).toEqual([]);
    });

    test('returns hosts with recent heartbeats', async () => {
      await ledger.recordStorageHeartbeat('alice', [CID_A], 50, 100);
      await ledger.recordStorageHeartbeat('bob', [CID_B], 75, 95);
      await ledger.recordStorageHeartbeat('carol', [CID_C], 25, 10); // old

      const active = stateStore.getActiveStorageHosts(100, 50);
      const names = active.map((h) => h.host).sort();
      expect(names).toEqual(['alice', 'bob']);
    });

    test('carol still appears if window is wide enough', async () => {
      await ledger.recordStorageHeartbeat('alice', [CID_A], 50, 100);
      await ledger.recordStorageHeartbeat('carol', [CID_C], 25, 10);
      const active = stateStore.getActiveStorageHosts(100, 200);
      expect(active.length).toBe(2);
    });
  });

  describe('dedupe behavior', () => {
    test('duplicate heartbeat in same epoch from same host is deduped', async () => {
      await ledger.recordStorageHeartbeat('alice', [CID_A], 50, 5);
      await ledger.recordStorageHeartbeat('alice', [CID_A], 50, 5); // duplicate
      const hb = stateStore.getStorageHeartbeat('alice');
      expect(hb.total_heartbeats).toBe(1);
    });

    test('different epochs from same host are not deduped', async () => {
      await ledger.recordStorageHeartbeat('alice', [CID_A], 50, 5);
      await ledger.recordStorageHeartbeat('alice', [CID_A], 50, 6);
      const hb = stateStore.getStorageHeartbeat('alice');
      expect(hb.total_heartbeats).toBe(2);
    });
  });
});

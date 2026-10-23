"use strict";

/**
 * Blob payouts two-tier tests — v2.11.2-delta
 *
 * Verifies the v2.11.2 70/20/9/1 split, bandwidth × uptime weighting
 * for active hosts, pure uptime weighting for cold hosts, and the
 * critical invariant that low challenge success rate REDUCES payout
 * but NEVER slashes stake (see feedback_storage_no_slash.md).
 */

const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
}));

const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');
const payouts = require('../src/services/blobPayoutsTwoTier');
const epochBandwidth = require('../src/services/epochBandwidth');

const CID_A = 'a'.repeat(64);
const CID_B = 'b'.repeat(64);

describe('blobPayouts two-tier (v2.11.2-delta)', () => {
  beforeEach(() => {
    stateStore.resetAll();
    epochBandwidth.resetAll();
    ledger.flushPendingEntries();
    mockMempoolSubmit.mockReset();
    mockMempoolSubmit.mockReturnValue({ accepted: true });
    ['alice', 'bob', 'carol', 'shindevlin'].forEach(a => epochBandwidth.seedForTest(a));
    // Seed uploader with HONE
    stateStore.applyEntry({
      type: 'FAUCET',
      from: 'hone_genesis',
      to: 'shindevlin',
      token: 'HONE',
      amount: 100000,
      epoch: 0,
      timestamp: 1,
    });
  });

  describe('computePayouts (pure, with context)', () => {
    test('two-tier split: 70/20/9/1', () => {
      const blob = {
        cid: CID_A,
        payment_hone: 1000,
        active_hosts: ['alice'],
        cold_hosts: ['bob'],
        bytes_served_by_host: { alice: 1000 },
      };
      const result = payouts.computePayouts(blob, {
        uptime_by_host: { alice: 1.0, bob: 1.0 },
        success_by_host: { alice: 1.0, bob: 1.0 },
      });
      expect(result.version).toBe('2.11.2');
      expect(result.active_pool).toBe(700);
      expect(result.cold_pool).toBe(200);
      expect(result.recycle_pool).toBe(90);
      expect(result.reputation_pool).toBe(10);
    });

    test('active host with all bytes + full uptime gets entire active pool', () => {
      const blob = {
        cid: CID_A,
        payment_hone: 1000,
        active_hosts: ['alice'],
        cold_hosts: [],
        bytes_served_by_host: { alice: 1000 },
      };
      const result = payouts.computePayouts(blob, {
        uptime_by_host: { alice: 1.0 },
        success_by_host: { alice: 1.0 },
      });
      // Active pool = 700, alice gets 100% (1.0 bandwidth weight + 1.0 uptime weight, composite 1.0)
      expect(result.active_payouts[0].amount_hone).toBeCloseTo(700, 5);
    });

    test('cold host with full uptime gets entire cold pool', () => {
      const blob = {
        cid: CID_A,
        payment_hone: 1000,
        active_hosts: [],
        cold_hosts: ['bob'],
      };
      const result = payouts.computePayouts(blob, {
        uptime_by_host: { bob: 1.0 },
        success_by_host: { bob: 1.0 },
      });
      // Cold pool = 200, bob gets 100%
      expect(result.cold_payouts[0].amount_hone).toBeCloseTo(200, 5);
    });

    test('active host with 70% bandwidth + 30% uptime split works correctly', () => {
      const blob = {
        cid: CID_A,
        payment_hone: 1000,
        active_hosts: ['alice', 'bob'],
        cold_hosts: [],
        bytes_served_by_host: { alice: 1000, bob: 0 }, // alice serves all bytes
      };
      const result = payouts.computePayouts(blob, {
        // alice low uptime, bob high uptime
        uptime_by_host: { alice: 0.2, bob: 0.8 },
        success_by_host: { alice: 1.0, bob: 1.0 },
      });
      // alice: 0.7 * (1000/1000) + 0.3 * (0.2/1.0) = 0.7 + 0.06 = 0.76
      // bob:   0.7 * (0/1000) + 0.3 * (0.8/1.0) = 0 + 0.24 = 0.24
      // alice gets 700 * 0.76 = 532
      // bob   gets 700 * 0.24 = 168
      const alice = result.active_payouts.find((p) => p.host === 'alice');
      const bob = result.active_payouts.find((p) => p.host === 'bob');
      expect(alice.amount_hone).toBeCloseTo(532, 3);
      expect(bob.amount_hone).toBeCloseTo(168, 3);
    });

    test('cold host split by uptime alone', () => {
      const blob = {
        cid: CID_A,
        payment_hone: 1000,
        active_hosts: [],
        cold_hosts: ['alice', 'bob', 'carol'],
      };
      const result = payouts.computePayouts(blob, {
        uptime_by_host: { alice: 0.5, bob: 1.0, carol: 0.5 },
        success_by_host: { alice: 1.0, bob: 1.0, carol: 1.0 },
      });
      // Total cold uptime = 2.0
      // alice: 200 * (0.5 / 2.0) = 50
      // bob:   200 * (1.0 / 2.0) = 100
      // carol: 200 * (0.5 / 2.0) = 50
      const byHost = {};
      result.cold_payouts.forEach((p) => { byHost[p.host] = p.amount_hone; });
      expect(byHost.alice).toBeCloseTo(50, 3);
      expect(byHost.bob).toBeCloseTo(100, 3);
      expect(byHost.carol).toBeCloseTo(50, 3);
    });

    test('challenge success rate multiplies the payout', () => {
      const blob = {
        cid: CID_A,
        payment_hone: 1000,
        active_hosts: [],
        cold_hosts: ['alice'],
      };
      // 50% success → half payout
      const result = payouts.computePayouts(blob, {
        uptime_by_host: { alice: 1.0 },
        success_by_host: { alice: 0.5 },
      });
      // Cold pool = 200, alice weight = 1.0, success = 0.5 → 100
      expect(result.cold_payouts[0].amount_hone).toBeCloseTo(100, 5);
    });

    test('0% challenge success → 0 payout (pay-for-delivery)', () => {
      const blob = {
        cid: CID_A,
        payment_hone: 1000,
        active_hosts: [],
        cold_hosts: ['alice'],
      };
      const result = payouts.computePayouts(blob, {
        uptime_by_host: { alice: 1.0 },
        success_by_host: { alice: 0.0 },
      });
      expect(result.cold_payouts[0].amount_hone).toBe(0);
    });

    test('zero uptime → zero payout', () => {
      const blob = {
        cid: CID_A,
        payment_hone: 1000,
        active_hosts: [],
        cold_hosts: ['alice'],
      };
      const result = payouts.computePayouts(blob, {
        uptime_by_host: { alice: 0.0 },
        success_by_host: { alice: 1.0 },
      });
      expect(result.cold_payouts[0].amount_hone).toBe(0);
    });

    test('missing context factors default to 1.0 (full credit)', () => {
      const blob = {
        cid: CID_A,
        payment_hone: 1000,
        active_hosts: [],
        cold_hosts: ['alice'],
      };
      const result = payouts.computePayouts(blob, {});
      expect(result.cold_payouts[0].amount_hone).toBeCloseTo(200, 5);
    });
  });

  describe('settlePayouts end-to-end (with real stateStore)', () => {
    test('two-tier commit: active gets 70% active pool, cold gets 20% cold pool', async () => {
      // Seed uptime via heartbeats (20 heartbeats at every 5 epochs = full uptime)
      for (let e = 1; e <= 100; e += 5) {
        await ledger.recordStorageHeartbeat('alice', [CID_A], 50, e);
        await ledger.recordStorageHeartbeat('bob', [CID_A], 50, e);
      }

      await ledger.recordBlobStoreCommit(
        'shindevlin',
        CID_A,
        1000,
        {
          active_hosts: ['alice'],
          cold_hosts: ['bob'],
          duration_epochs: 100,
          payment_hone: 1000,
        },
        1
      );
      await ledger.recordBlobServeProof('alice', CID_A, 5000, 10, null, 2);

      const blob = stateStore.getBlobCommit(CID_A);
      await payouts.settlePayouts(blob, {
        uploader: 'shindevlin',
        epoch: 100,
        current_epoch: 100,
      });

      // alice = active, got all bytes + full uptime → 700
      // bob = cold, full uptime → 200
      expect(stateStore.getBalance('alice', 'HONE')).toBeCloseTo(700, 3);
      expect(stateStore.getBalance('bob', 'HONE')).toBeCloseTo(200, 3);
      expect(stateStore.getBalance('hone_recycle', 'HONE')).toBeCloseTo(90, 3);
      expect(stateStore.getBalance('hone_reputation_pool', 'HONE')).toBeCloseTo(10, 3);
    });

    test('failed challenges reduce payout but do NOT slash balance/stake', async () => {
      // Seed alice with starting balance + stake (so we can verify stake unchanged)
      stateStore.applyEntry({
        type: 'FAUCET',
        from: 'hone_genesis',
        to: 'alice',
        token: 'HONE',
        amount: 1000,
        epoch: 0,
        timestamp: 99,
      });
      await ledger.recordStake('alice', 50, 'storage', 1);
      const aliceBalanceBefore = stateStore.getBalance('alice', 'HONE');
      const stakeBeforeSettle = stateStore.getStakePool('alice').total_staked;

      // Seed uptime: 20 heartbeats over 100 epochs = 1.0 uptime
      for (let e = 1; e <= 100; e += 5) {
        await ledger.recordStorageHeartbeat('alice', [CID_A], 50, e);
      }

      // alice has 50% challenge success rate (1 pass, 1 fail)
      const HASH_OK = 'a'.repeat(64);
      const HASH_BAD = 'b'.repeat(64);
      await ledger.recordBlobChallenge('v1', 'c1', 'alice', CID_A, 0, 100, HASH_OK, 10);
      await ledger.recordBlobChallengeResponse('alice', 'c1', HASH_OK, 11);
      await ledger.recordBlobChallenge('v1', 'c2', 'alice', CID_A, 100, 100, HASH_OK, 12);
      await ledger.recordBlobChallengeResponse('alice', 'c2', HASH_BAD, 13);

      expect(stateStore.getChallengeSuccessRate('alice')).toBe(0.5);

      await ledger.recordBlobStoreCommit(
        'shindevlin',
        CID_A,
        1000,
        {
          active_hosts: [],
          cold_hosts: ['alice'],
          duration_epochs: 100,
          payment_hone: 1000,
        },
        14
      );

      const blob = stateStore.getBlobCommit(CID_A);
      await payouts.settlePayouts(blob, {
        uploader: 'shindevlin',
        epoch: 100,
        current_epoch: 100,
      });

      // Payout should be 200 * 1.0 * 0.5 = 100 (half of cold pool due to 50% success)
      // alice balance: starting (1000 - 50 stake) + 100 payout = 1050
      expect(stateStore.getBalance('alice', 'HONE')).toBeCloseTo(aliceBalanceBefore + 100, 3);

      // Stake unchanged (NOT slashed)
      expect(stateStore.getStakePool('alice').total_staked).toBe(stakeBeforeSettle);
    });

    test('zero uptime host earns zero, unused pool stays unpaid', async () => {
      // alice has zero heartbeats → zero uptime
      await ledger.recordBlobStoreCommit(
        'shindevlin',
        CID_A,
        1000,
        {
          active_hosts: [],
          cold_hosts: ['alice'],
          duration_epochs: 100,
          payment_hone: 1000,
        },
        14
      );

      const blob = stateStore.getBlobCommit(CID_A);
      const uploaderBefore = stateStore.getBalance('shindevlin', 'HONE');
      const result = await payouts.settlePayouts(blob, {
        uploader: 'shindevlin',
        epoch: 100,
        current_epoch: 100,
      });

      // alice earned nothing
      expect(stateStore.getBalance('alice', 'HONE')).toBe(0);
      // Recycle + reputation still paid
      expect(stateStore.getBalance('hone_recycle', 'HONE')).toBeCloseTo(90, 3);
      expect(stateStore.getBalance('hone_reputation_pool', 'HONE')).toBeCloseTo(10, 3);
      // Uploader only paid the recycle + reputation shares (~100), not the full pool
      const uploaderAfter = stateStore.getBalance('shindevlin', 'HONE');
      expect(uploaderBefore - uploaderAfter).toBeCloseTo(100, 3);
    });

    test('multi-host two-tier end-to-end with mixed uptime', async () => {
      // Seed varying uptimes via heartbeat counts
      // alice: 20 heartbeats (100% uptime)
      // bob: 10 heartbeats (50% uptime)
      // carol: 5 heartbeats (25% uptime)
      for (let e = 1; e <= 100; e += 5) {
        await ledger.recordStorageHeartbeat('alice', [CID_A], 100, e);
      }
      for (let e = 1; e <= 50; e += 5) {
        await ledger.recordStorageHeartbeat('bob', [CID_A], 100, e);
      }
      for (let e = 1; e <= 25; e += 5) {
        await ledger.recordStorageHeartbeat('carol', [CID_A], 100, e);
      }

      await ledger.recordBlobStoreCommit(
        'shindevlin',
        CID_A,
        1000,
        {
          active_hosts: ['alice'],
          cold_hosts: ['bob', 'carol'],
          duration_epochs: 100,
          payment_hone: 1000,
        },
        1
      );
      await ledger.recordBlobServeProof('alice', CID_A, 10000, 20, null, 2);

      const blob = stateStore.getBlobCommit(CID_A);
      const result = await payouts.settlePayouts(blob, {
        uploader: 'shindevlin',
        epoch: 100,
        current_epoch: 100,
      });

      // alice (active, sole): full active pool 700 (1.0 bandwidth + 1.0 uptime composite × 1.0 success)
      expect(stateStore.getBalance('alice', 'HONE')).toBeCloseTo(700, 3);
      // bob (cold, 0.5 uptime), carol (cold, 0.25 uptime)
      // Total cold uptime = 0.75
      // bob:   200 * (0.5/0.75) ≈ 133.33
      // carol: 200 * (0.25/0.75) ≈ 66.67
      expect(stateStore.getBalance('bob', 'HONE')).toBeCloseTo(133.33, 1);
      expect(stateStore.getBalance('carol', 'HONE')).toBeCloseTo(66.67, 1);
      expect(stateStore.getBalance('hone_recycle', 'HONE')).toBeCloseTo(90, 3);
      expect(stateStore.getBalance('hone_reputation_pool', 'HONE')).toBeCloseTo(10, 3);
    });
  });
});

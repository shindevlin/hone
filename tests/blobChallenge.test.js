"use strict";

/**
 * BLOB_CHALLENGE tests — v2.11.2
 *
 * Challenge-response protocol for storage hosts. Critical design note:
 * failures do NOT slash stake. See feedback_storage_no_slash.md. Tests
 * verify that failed challenges only affect payout weight + reputation,
 * never the host's BTCPC balance or staked amount.
 */

const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
}));

const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');

const CID_A = 'a'.repeat(64);
const CID_B = 'b'.repeat(64);
const HASH_CORRECT = 'c'.repeat(64);
const HASH_WRONG = 'd'.repeat(64);

describe('BLOB_CHALLENGE (v2.11.2)', () => {
  beforeEach(() => {
    stateStore.resetAll();
    ledger.flushPendingEntries();
    mockMempoolSubmit.mockReset();
    mockMempoolSubmit.mockReturnValue({ accepted: true });
  });

  describe('recordBlobChallenge validation', () => {
    test('requires challenger', async () => {
      await expect(
        ledger.recordBlobChallenge(null, 'c1', 'alice', CID_A, 0, 100, null, 1)
      ).rejects.toThrow('challenger required');
    });

    test('requires challengeId', async () => {
      await expect(
        ledger.recordBlobChallenge('v1', null, 'alice', CID_A, 0, 100, null, 1)
      ).rejects.toThrow('challengeId required');
    });

    test('requires host', async () => {
      await expect(
        ledger.recordBlobChallenge('v1', 'c1', null, CID_A, 0, 100, null, 1)
      ).rejects.toThrow('host required');
    });

    test('requires valid CID', async () => {
      await expect(
        ledger.recordBlobChallenge('v1', 'c1', 'alice', 'bad', 0, 100, null, 1)
      ).rejects.toThrow('64-char');
    });

    test('requires valid byte range', async () => {
      await expect(
        ledger.recordBlobChallenge('v1', 'c1', 'alice', CID_A, -1, 100, null, 1)
      ).rejects.toThrow('byteStart');
      await expect(
        ledger.recordBlobChallenge('v1', 'c1', 'alice', CID_A, 0, 0, null, 1)
      ).rejects.toThrow('byteLength');
    });
  });

  describe('challenge dispatcher', () => {
    test('creates challenge in pending state', async () => {
      await ledger.recordBlobChallenge('v1', 'chall-1', 'alice', CID_A, 0, 100, null, 5);
      const c = stateStore.getBlobChallenge('chall-1');
      expect(c).toBeTruthy();
      expect(c.challenger).toBe('v1');
      expect(c.host).toBe('alice');
      expect(c.cid).toBe(CID_A);
      expect(c.status).toBe('pending');
      expect(c.issued_epoch).toBe(5);
    });

    test('increments total_issued on the host stats', async () => {
      await ledger.recordBlobChallenge('v1', 'chall-1', 'alice', CID_A, 0, 100, null, 5);
      const stats = stateStore.getBlobChallengeStats('alice');
      expect(stats.total_issued).toBe(1);
      expect(stats.total_passed).toBe(0);
      expect(stats.total_failed).toBe(0);
    });

    test('duplicate challenge_id is deduped (not overwritten)', async () => {
      await ledger.recordBlobChallenge('v1', 'chall-1', 'alice', CID_A, 0, 100, null, 5);
      await ledger.recordBlobChallenge('v2', 'chall-1', 'bob', CID_B, 0, 100, null, 6);
      const c = stateStore.getBlobChallenge('chall-1');
      expect(c.host).toBe('alice'); // first one won
      expect(c.challenger).toBe('v1');
    });
  });

  describe('auto-resolving with expected_hash', () => {
    test('response matching expected_hash → passed', async () => {
      await ledger.recordBlobChallenge('v1', 'chall-1', 'alice', CID_A, 0, 100, HASH_CORRECT, 5);
      await ledger.recordBlobChallengeResponse('alice', 'chall-1', HASH_CORRECT, 6);
      const c = stateStore.getBlobChallenge('chall-1');
      expect(c.status).toBe('passed');
      expect(c.response_hash).toBe(HASH_CORRECT);
      expect(c.response_epoch).toBe(6);

      const stats = stateStore.getBlobChallengeStats('alice');
      expect(stats.total_passed).toBe(1);
      expect(stats.total_failed).toBe(0);
    });

    test('response mismatching expected_hash → failed_mismatch', async () => {
      await ledger.recordBlobChallenge('v1', 'chall-1', 'alice', CID_A, 0, 100, HASH_CORRECT, 5);
      await ledger.recordBlobChallengeResponse('alice', 'chall-1', HASH_WRONG, 6);
      const c = stateStore.getBlobChallenge('chall-1');
      expect(c.status).toBe('failed_mismatch');

      const stats = stateStore.getBlobChallengeStats('alice');
      expect(stats.total_passed).toBe(0);
      expect(stats.total_failed).toBe(1);
    });

    test('response from wrong host is rejected', async () => {
      await ledger.recordBlobChallenge('v1', 'chall-1', 'alice', CID_A, 0, 100, HASH_CORRECT, 5);
      await ledger.recordBlobChallengeResponse('mallory', 'chall-1', HASH_CORRECT, 6);
      const c = stateStore.getBlobChallenge('chall-1');
      expect(c.status).toBe('pending');
      // Stats shouldn't bump either
      const stats = stateStore.getBlobChallengeStats('mallory');
      expect(stats.total_passed).toBe(0);
    });
  });

  describe('verifier-audited resolution (no expected_hash)', () => {
    test('response → responded → result passed', async () => {
      await ledger.recordBlobChallenge('v1', 'chall-1', 'alice', CID_A, 0, 100, null, 5);
      await ledger.recordBlobChallengeResponse('alice', 'chall-1', HASH_CORRECT, 6);
      expect(stateStore.getBlobChallenge('chall-1').status).toBe('responded');

      await ledger.recordBlobChallengeResult('v1', 'chall-1', true, 7);
      expect(stateStore.getBlobChallenge('chall-1').status).toBe('passed');
      expect(stateStore.getBlobChallengeStats('alice').total_passed).toBe(1);
    });

    test('response → responded → result failed', async () => {
      await ledger.recordBlobChallenge('v1', 'chall-1', 'alice', CID_A, 0, 100, null, 5);
      await ledger.recordBlobChallengeResponse('alice', 'chall-1', HASH_WRONG, 6);
      await ledger.recordBlobChallengeResult('v1', 'chall-1', false, 7);
      expect(stateStore.getBlobChallenge('chall-1').status).toBe('failed_mismatch');
      expect(stateStore.getBlobChallengeStats('alice').total_failed).toBe(1);
    });

    test('only original challenger can record result', async () => {
      await ledger.recordBlobChallenge('v1', 'chall-1', 'alice', CID_A, 0, 100, null, 5);
      await ledger.recordBlobChallengeResponse('alice', 'chall-1', HASH_CORRECT, 6);
      await ledger.recordBlobChallengeResult('v2', 'chall-1', true, 7); // wrong verifier
      expect(stateStore.getBlobChallenge('chall-1').status).toBe('responded');
    });
  });

  describe('timeout handling', () => {
    test('verifier records timeout → failed_timeout', async () => {
      await ledger.recordBlobChallenge('v1', 'chall-1', 'alice', CID_A, 0, 100, null, 5);
      // Host never responds, verifier declares timeout 3 epochs later
      await ledger.recordBlobChallengeTimeout('v1', 'chall-1', 8);
      const c = stateStore.getBlobChallenge('chall-1');
      expect(c.status).toBe('failed_timeout');
      expect(stateStore.getBlobChallengeStats('alice').total_failed).toBe(1);
    });

    test('timeout on already-responded challenge is a no-op', async () => {
      await ledger.recordBlobChallenge('v1', 'chall-1', 'alice', CID_A, 0, 100, HASH_CORRECT, 5);
      await ledger.recordBlobChallengeResponse('alice', 'chall-1', HASH_CORRECT, 6);
      await ledger.recordBlobChallengeTimeout('v1', 'chall-1', 8);
      expect(stateStore.getBlobChallenge('chall-1').status).toBe('passed');
    });
  });

  describe('NO SLASHING (the whole point)', () => {
    test('failed challenge does not change host balance or stake', async () => {
      // Give alice some BTCPC and stake
      stateStore.applyEntry({
        type: 'FAUCET',
        from: 'btcpc_genesis',
        to: 'alice',
        token: 'BTCPC',
        amount: 1000,
        epoch: 0,
        timestamp: 1,
      });
      await ledger.recordStake('alice', 100, 'storage', 1);

      const balanceBefore = stateStore.getBalance('alice', 'BTCPC');
      const stakeBefore = stateStore.getStakePool('alice');

      // Issue 10 failed challenges in a row
      for (let i = 0; i < 10; i++) {
        await ledger.recordBlobChallenge(
          'v1',
          'chall-' + i,
          'alice',
          CID_A,
          0,
          100,
          HASH_CORRECT,
          5 + i
        );
        await ledger.recordBlobChallengeResponse('alice', 'chall-' + i, HASH_WRONG, 6 + i);
      }

      const balanceAfter = stateStore.getBalance('alice', 'BTCPC');
      const stakeAfter = stateStore.getStakePool('alice');

      // Balance unchanged
      expect(balanceAfter).toBe(balanceBefore);
      // Stake unchanged
      expect(stakeAfter.total_staked).toBe(stakeBefore.total_staked);
      // But stats reflect the failures
      expect(stateStore.getBlobChallengeStats('alice').total_failed).toBe(10);
    });

    test('timeout failures also do not slash', async () => {
      stateStore.applyEntry({
        type: 'FAUCET',
        from: 'btcpc_genesis',
        to: 'alice',
        token: 'BTCPC',
        amount: 1000,
        epoch: 0,
        timestamp: 1,
      });
      await ledger.recordStake('alice', 100, 'storage', 1);
      const balanceBefore = stateStore.getBalance('alice', 'BTCPC');

      for (let i = 0; i < 5; i++) {
        await ledger.recordBlobChallenge(
          'v1',
          'timeout-' + i,
          'alice',
          CID_A,
          0,
          100,
          null,
          5 + i
        );
        await ledger.recordBlobChallengeTimeout('v1', 'timeout-' + i, 10 + i);
      }

      expect(stateStore.getBalance('alice', 'BTCPC')).toBe(balanceBefore);
      expect(stateStore.getBlobChallengeStats('alice').total_failed).toBe(5);
    });
  });

  describe('getChallengeSuccessRate', () => {
    test('returns 1.0 for host with no challenges (benefit of the doubt)', () => {
      expect(stateStore.getChallengeSuccessRate('new-host')).toBe(1.0);
    });

    test('100% success returns 1.0', async () => {
      for (let i = 0; i < 5; i++) {
        await ledger.recordBlobChallenge('v1', 'pc-' + i, 'alice', CID_A, 0, 100, HASH_CORRECT, 5 + i);
        await ledger.recordBlobChallengeResponse('alice', 'pc-' + i, HASH_CORRECT, 6 + i);
      }
      expect(stateStore.getChallengeSuccessRate('alice')).toBe(1.0);
    });

    test('mixed success returns fraction', async () => {
      // 3 passes, 2 failures
      for (let i = 0; i < 3; i++) {
        await ledger.recordBlobChallenge('v1', 'pass-' + i, 'alice', CID_A, 0, 100, HASH_CORRECT, 5 + i);
        await ledger.recordBlobChallengeResponse('alice', 'pass-' + i, HASH_CORRECT, 6 + i);
      }
      for (let i = 0; i < 2; i++) {
        await ledger.recordBlobChallenge('v1', 'fail-' + i, 'alice', CID_A, 0, 100, HASH_CORRECT, 20 + i);
        await ledger.recordBlobChallengeResponse('alice', 'fail-' + i, HASH_WRONG, 21 + i);
      }
      expect(stateStore.getChallengeSuccessRate('alice')).toBeCloseTo(0.6, 5);
    });
  });

  describe('getPendingChallengesForHost', () => {
    test('returns only pending challenges for the given host', async () => {
      await ledger.recordBlobChallenge('v1', 'p1', 'alice', CID_A, 0, 100, null, 5);
      await ledger.recordBlobChallenge('v1', 'p2', 'alice', CID_B, 0, 100, null, 5);
      await ledger.recordBlobChallenge('v1', 'p3', 'bob', CID_A, 0, 100, null, 5);
      // Respond to p1
      await ledger.recordBlobChallengeResponse('alice', 'p1', HASH_CORRECT, 6);
      await ledger.recordBlobChallengeResult('v1', 'p1', true, 7);

      const pending = stateStore.getPendingChallengesForHost('alice');
      expect(pending.length).toBe(1);
      expect(pending[0].challenge_id).toBe('p2');
    });
  });
});

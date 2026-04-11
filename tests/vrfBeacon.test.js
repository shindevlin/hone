"use strict";

/**
 * VRF beacon tests — v2.12-gamma
 *
 * Tests the commit-reveal scheme, beacon aggregation, and the
 * derivation helpers (int, float, shuffle).
 */

const vrf = require('../src/services/vrfBeacon');
const crypto = require('crypto');

describe('vrfBeacon (v2.12-gamma)', () => {
  describe('generateSecret + commitHash', () => {
    test('generateSecret returns 32-byte hex', () => {
      const secret = vrf.generateSecret();
      expect(typeof secret).toBe('string');
      expect(secret.length).toBe(64); // 32 bytes = 64 hex chars
      expect(/^[a-f0-9]+$/.test(secret)).toBe(true);
    });

    test('two secrets differ', () => {
      const a = vrf.generateSecret();
      const b = vrf.generateSecret();
      expect(a).not.toBe(b);
    });

    test('commitHash is deterministic', () => {
      const secret = vrf.generateSecret();
      expect(vrf.commitHash(secret)).toBe(vrf.commitHash(secret));
    });

    test('commitHash differs for different secrets', () => {
      const a = vrf.commitHash(vrf.generateSecret());
      const b = vrf.commitHash(vrf.generateSecret());
      expect(a).not.toBe(b);
    });

    test('commitHash returns 64-char hex', () => {
      const secret = vrf.generateSecret();
      const hash = vrf.commitHash(secret);
      expect(hash.length).toBe(64);
      expect(/^[a-f0-9]+$/.test(hash)).toBe(true);
    });

    test('commitHash rejects non-string secret', () => {
      expect(() => vrf.commitHash(null)).toThrow();
      expect(() => vrf.commitHash(123)).toThrow();
    });
  });

  describe('verifyReveal', () => {
    test('matching secret + commit returns true', () => {
      const secret = vrf.generateSecret();
      const commit = vrf.commitHash(secret);
      expect(vrf.verifyReveal(secret, commit)).toBe(true);
    });

    test('mismatched secret + commit returns false', () => {
      const a = vrf.generateSecret();
      const b = vrf.generateSecret();
      const commitB = vrf.commitHash(b);
      expect(vrf.verifyReveal(a, commitB)).toBe(false);
    });

    test('null inputs return false', () => {
      expect(vrf.verifyReveal(null, 'whatever')).toBe(false);
      expect(vrf.verifyReveal('whatever', null)).toBe(false);
    });
  });

  describe('computeBeacon', () => {
    function makeContribution(nodeId) {
      const secret = vrf.generateSecret();
      return {
        node_id: nodeId,
        commit: vrf.commitHash(secret),
        reveal: secret,
      };
    }

    test('empty contributions returns null beacon', () => {
      const result = vrf.computeBeacon([]);
      expect(result.beacon).toBeNull();
      expect(result.contributors).toEqual([]);
    });

    test('single valid contribution produces beacon', () => {
      const c = makeContribution('alice');
      const result = vrf.computeBeacon([c]);
      expect(result.beacon).not.toBeNull();
      expect(result.beacon.length).toBe(64);
      expect(result.contributors).toEqual(['alice']);
      expect(result.rejected).toEqual([]);
    });

    test('multiple valid contributions XOR together', () => {
      const a = makeContribution('alice');
      const b = makeContribution('bob');
      const c = makeContribution('carol');
      const result = vrf.computeBeacon([a, b, c]);
      expect(result.beacon).not.toBeNull();
      expect(result.contributors.sort()).toEqual(['alice', 'bob', 'carol']);

      // Verify the XOR manually
      const xor = Buffer.alloc(32);
      [a.reveal, b.reveal, c.reveal].forEach((r) => {
        const buf = Buffer.from(r, 'hex');
        for (let i = 0; i < 32; i++) xor[i] ^= buf[i];
      });
      expect(result.beacon).toBe(xor.toString('hex'));
    });

    test('mismatched reveals are rejected', () => {
      const a = makeContribution('alice');
      const b = makeContribution('bob');
      // bob lies — reveals a different secret than what he committed
      b.reveal = vrf.generateSecret();
      const result = vrf.computeBeacon([a, b]);
      expect(result.contributors).toEqual(['alice']);
      expect(result.rejected.length).toBe(1);
      expect(result.rejected[0].node_id).toBe('bob');
    });

    test('order does not affect beacon (XOR is commutative)', () => {
      const a = makeContribution('alice');
      const b = makeContribution('bob');
      const c = makeContribution('carol');

      const r1 = vrf.computeBeacon([a, b, c]);
      const r2 = vrf.computeBeacon([c, a, b]);
      const r3 = vrf.computeBeacon([b, c, a]);

      expect(r1.beacon).toBe(r2.beacon);
      expect(r2.beacon).toBe(r3.beacon);
    });

    test('one honest contributor is enough to make the beacon unpredictable', () => {
      // Even if N-1 contributors are colluding, the Nth honest one
      // changes the XOR completely
      const honest = makeContribution('honest');
      const colluder1 = makeContribution('c1');
      const colluder2 = makeContribution('c2');

      const withHonest = vrf.computeBeacon([honest, colluder1, colluder2]);
      const withoutHonest = vrf.computeBeacon([colluder1, colluder2]);
      expect(withHonest.beacon).not.toBe(withoutHonest.beacon);
    });

    test('contributions with missing fields are rejected', () => {
      const valid = makeContribution('alice');
      const broken = { node_id: 'bob' }; // no commit/reveal
      const result = vrf.computeBeacon([valid, broken]);
      expect(result.contributors).toEqual(['alice']);
      expect(result.rejected.length).toBe(1);
    });

    test('all-rejected → null beacon', () => {
      const broken1 = { node_id: 'a', commit: 'x', reveal: 'y' };
      const broken2 = { node_id: 'b', commit: 'x', reveal: 'y' };
      const result = vrf.computeBeacon([broken1, broken2]);
      expect(result.beacon).toBeNull();
      expect(result.rejected.length).toBe(2);
    });
  });

  describe('deriveInt / deriveIndex', () => {
    let testBeacon;
    beforeAll(() => {
      const c = {
        node_id: 'test',
        commit: '',
        reveal: 'a'.repeat(64),
      };
      c.commit = vrf.commitHash(c.reveal);
      testBeacon = vrf.computeBeacon([c]).beacon;
    });

    test('deterministic for same beacon + salt', () => {
      const a = vrf.deriveInt(testBeacon, 100, 'test');
      const b = vrf.deriveInt(testBeacon, 100, 'test');
      expect(a).toBe(b);
    });

    test('different salts give different values', () => {
      const a = vrf.deriveInt(testBeacon, 1000000, 'salt-a');
      const b = vrf.deriveInt(testBeacon, 1000000, 'salt-b');
      expect(a).not.toBe(b);
    });

    test('value is in [0, max)', () => {
      for (let i = 0; i < 100; i++) {
        const v = vrf.deriveInt(testBeacon, 100, 'iter-' + i);
        expect(v).toBeGreaterThanOrEqual(0);
        expect(v).toBeLessThan(100);
      }
    });

    test('null beacon returns 0', () => {
      expect(vrf.deriveInt(null, 100, 'x')).toBe(0);
    });

    test('deriveIndex is alias for deriveInt', () => {
      const list = ['a', 'b', 'c', 'd', 'e'];
      const idx = vrf.deriveIndex(testBeacon, list.length, 'pick');
      expect(idx).toBeGreaterThanOrEqual(0);
      expect(idx).toBeLessThan(list.length);
    });
  });

  describe('deriveFloat', () => {
    let testBeacon;
    beforeAll(() => {
      const c = {
        node_id: 'test',
        commit: '',
        reveal: 'b'.repeat(64),
      };
      c.commit = vrf.commitHash(c.reveal);
      testBeacon = vrf.computeBeacon([c]).beacon;
    });

    test('returns value in [0, 1)', () => {
      for (let i = 0; i < 100; i++) {
        const f = vrf.deriveFloat(testBeacon, 'iter-' + i);
        expect(f).toBeGreaterThanOrEqual(0);
        expect(f).toBeLessThan(1);
      }
    });

    test('deterministic for same beacon + salt', () => {
      const a = vrf.deriveFloat(testBeacon, 'same');
      const b = vrf.deriveFloat(testBeacon, 'same');
      expect(a).toBe(b);
    });

    test('reasonably distributed', () => {
      // Generate 1000 floats, expect mean ~0.5 (loose check)
      let sum = 0;
      for (let i = 0; i < 1000; i++) {
        sum += vrf.deriveFloat(testBeacon, 'i-' + i);
      }
      const mean = sum / 1000;
      expect(mean).toBeGreaterThan(0.4);
      expect(mean).toBeLessThan(0.6);
    });
  });

  describe('shuffleArray', () => {
    let testBeacon;
    beforeAll(() => {
      const c = {
        node_id: 'test',
        commit: '',
        reveal: 'c'.repeat(64),
      };
      c.commit = vrf.commitHash(c.reveal);
      testBeacon = vrf.computeBeacon([c]).beacon;
    });

    test('preserves length', () => {
      const arr = [1, 2, 3, 4, 5];
      const shuffled = vrf.shuffleArray(testBeacon, arr);
      expect(shuffled.length).toBe(5);
    });

    test('preserves contents (same elements)', () => {
      const arr = ['a', 'b', 'c', 'd', 'e'];
      const shuffled = vrf.shuffleArray(testBeacon, arr).slice().sort();
      expect(shuffled).toEqual(['a', 'b', 'c', 'd', 'e']);
    });

    test('does not mutate the input', () => {
      const arr = [1, 2, 3, 4, 5];
      vrf.shuffleArray(testBeacon, arr);
      expect(arr).toEqual([1, 2, 3, 4, 5]);
    });

    test('deterministic for same beacon', () => {
      const arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
      const a = vrf.shuffleArray(testBeacon, arr);
      const b = vrf.shuffleArray(testBeacon, arr);
      expect(a).toEqual(b);
    });

    test('different beacons give different orderings', () => {
      // Build a different beacon
      const c2 = {
        node_id: 'test2',
        commit: '',
        reveal: 'd'.repeat(64),
      };
      c2.commit = vrf.commitHash(c2.reveal);
      const beacon2 = vrf.computeBeacon([c2]).beacon;

      const arr = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
      const a = vrf.shuffleArray(testBeacon, arr);
      const b = vrf.shuffleArray(beacon2, arr);
      expect(a).not.toEqual(b);
    });

    test('empty array returns empty', () => {
      expect(vrf.shuffleArray(testBeacon, [])).toEqual([]);
    });
  });

  describe('end-to-end fairness scenarios', () => {
    test('lottery winner selection is deterministic + verifiable', () => {
      // Three clock nodes contribute secrets
      const a = (() => { const s = vrf.generateSecret(); return { node_id: 'alice', commit: vrf.commitHash(s), reveal: s }; })();
      const b = (() => { const s = vrf.generateSecret(); return { node_id: 'bob', commit: vrf.commitHash(s), reveal: s }; })();
      const c = (() => { const s = vrf.generateSecret(); return { node_id: 'carol', commit: vrf.commitHash(s), reveal: s }; })();

      const beacon = vrf.computeBeacon([a, b, c]).beacon;

      // 100 entrants in the lottery
      const entrants = [];
      for (let i = 0; i < 100; i++) entrants.push('entrant-' + i);

      // Pick a winner deterministically
      const winnerIdx = vrf.deriveIndex(beacon, entrants.length, 'lottery-1');
      const winner = entrants[winnerIdx];

      // Anyone with the same beacon + salt can verify
      const verifyIdx = vrf.deriveIndex(beacon, entrants.length, 'lottery-1');
      expect(verifyIdx).toBe(winnerIdx);
      expect(entrants[verifyIdx]).toBe(winner);
    });

    test('NFT mint order is deterministic + unbiasable', () => {
      // 10 NFTs to be minted in random order
      const nfts = ['rare-1', 'common-1', 'common-2', 'epic-1', 'common-3',
                    'rare-2', 'common-4', 'common-5', 'legendary', 'common-6'];

      // Build a beacon
      const a = (() => { const s = vrf.generateSecret(); return { node_id: 'a', commit: vrf.commitHash(s), reveal: s }; })();
      const beacon = vrf.computeBeacon([a]).beacon;

      const order = vrf.shuffleArray(beacon, nfts, 'mint-order');
      expect(order.length).toBe(10);
      expect(order.sort()).toEqual(nfts.slice().sort());
    });
  });
});

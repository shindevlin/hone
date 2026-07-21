"use strict";

/**
 * Anchor Submission tests — v2.16-alpha
 *
 * Covers anchor interval detection, payload computation, submission recording,
 * history retrieval, cost/reward queries, and append-only invariant.
 */

const anchor = require('../src/chain/anchorSubmission');

describe('anchorSubmission — constants', () => {
  it('L2 interval is 100 epochs', () => {
    expect(anchor.ANCHOR_INTERVALS.l2).toBe(100);
  });

  it('Ethereum interval is 1000 epochs', () => {
    expect(anchor.ANCHOR_INTERVALS.ethereum).toBe(1000);
  });

  it('Bitcoin interval is 10000 epochs', () => {
    expect(anchor.ANCHOR_INTERVALS.bitcoin).toBe(10000);
  });

  it('valid tiers are l2, ethereum, bitcoin', () => {
    expect(anchor.VALID_TIERS).toEqual(expect.arrayContaining(['l2', 'ethereum', 'bitcoin']));
    expect(anchor.VALID_TIERS).toHaveLength(3);
  });
});

describe('anchorSubmission — shouldAnchor', () => {
  it('returns true at L2 interval (multiple of 100)', () => {
    expect(anchor.shouldAnchor(100, 'l2')).toBe(true);
    expect(anchor.shouldAnchor(200, 'l2')).toBe(true);
    expect(anchor.shouldAnchor(10000, 'l2')).toBe(true);
  });

  it('returns false for non-multiple of 100 on L2', () => {
    expect(anchor.shouldAnchor(99, 'l2')).toBe(false);
    expect(anchor.shouldAnchor(101, 'l2')).toBe(false);
    expect(anchor.shouldAnchor(1, 'l2')).toBe(false);
  });

  it('returns true at Ethereum interval (multiple of 1000)', () => {
    expect(anchor.shouldAnchor(1000, 'ethereum')).toBe(true);
    expect(anchor.shouldAnchor(5000, 'ethereum')).toBe(true);
  });

  it('returns false for non-multiple on Ethereum', () => {
    expect(anchor.shouldAnchor(100, 'ethereum')).toBe(false);
    expect(anchor.shouldAnchor(999, 'ethereum')).toBe(false);
  });

  it('returns true at Bitcoin interval (multiple of 10000)', () => {
    expect(anchor.shouldAnchor(10000, 'bitcoin')).toBe(true);
    expect(anchor.shouldAnchor(20000, 'bitcoin')).toBe(true);
  });

  it('returns false at epoch 0 for all tiers', () => {
    expect(anchor.shouldAnchor(0, 'l2')).toBe(false);
    expect(anchor.shouldAnchor(0, 'ethereum')).toBe(false);
    expect(anchor.shouldAnchor(0, 'bitcoin')).toBe(false);
  });

  it('throws for invalid tier', () => {
    expect(() => anchor.shouldAnchor(100, 'solana')).toThrow(/invalid tier/);
  });

  it('Ethereum anchor epoch is also an L2 anchor point', () => {
    // 1000 is a multiple of both 100 and 1000
    expect(anchor.shouldAnchor(1000, 'l2')).toBe(true);
    expect(anchor.shouldAnchor(1000, 'ethereum')).toBe(true);
  });
});

describe('anchorSubmission — computeAnchorPayload', () => {
  it('returns required fields for a valid epoch', () => {
    const payload = anchor.computeAnchorPayload(1000);
    expect(payload).toHaveProperty('state_root');
    expect(payload).toHaveProperty('block_hash');
    expect(payload).toHaveProperty('epoch', 1000);
    expect(payload).toHaveProperty('timestamp');
    expect(payload).toHaveProperty('batch_size');
  });

  it('state_root and block_hash are non-empty strings', () => {
    const payload = anchor.computeAnchorPayload(500);
    expect(typeof payload.state_root).toBe('string');
    expect(payload.state_root.length).toBeGreaterThan(0);
    expect(typeof payload.block_hash).toBe('string');
    expect(payload.block_hash.length).toBeGreaterThan(0);
  });

  it('different epochs produce different hashes', () => {
    const p1 = anchor.computeAnchorPayload(1000);
    const p2 = anchor.computeAnchorPayload(2000);
    expect(p1.state_root).not.toBe(p2.state_root);
    expect(p1.block_hash).not.toBe(p2.block_hash);
  });

  it('batch_size is a positive integer', () => {
    const payload = anchor.computeAnchorPayload(500);
    expect(payload.batch_size).toBeGreaterThan(0);
    expect(Number.isInteger(payload.batch_size)).toBe(true);
  });

  it('throws for negative epoch', () => {
    expect(() => anchor.computeAnchorPayload(-1)).toThrow();
  });
});

describe('anchorSubmission — recordAnchorSubmission', () => {
  beforeEach(() => anchor.resetForTests());

  it('records a submission and returns a record', () => {
    const rec = anchor.recordAnchorSubmission('shindevlin', 'l2', 100, '0xTXHASH_L2_100');
    expect(rec.submitter).toBe('shindevlin');
    expect(rec.tier).toBe('l2');
    expect(rec.epoch).toBe(100);
    expect(rec.tx_hash).toBe('0xTXHASH_L2_100');
    expect(rec.reward_hone).toBeGreaterThan(0);
    expect(rec.submitted_at).toBeGreaterThan(0);
  });

  it('records an Ethereum anchor', () => {
    const rec = anchor.recordAnchorSubmission('natoshisakamoto', 'ethereum', 1000, '0xETH_TX');
    expect(rec.tier).toBe('ethereum');
    expect(rec.epoch).toBe(1000);
  });

  it('records a Bitcoin Deep Seal', () => {
    const rec = anchor.recordAnchorSubmission('shindevlin', 'bitcoin', 10000, '0xBTC_TXID');
    expect(rec.tier).toBe('bitcoin');
    expect(rec.epoch).toBe(10000);
  });

  it('append-only: cannot submit duplicate epoch anchor on same tier', () => {
    anchor.recordAnchorSubmission('shindevlin', 'l2', 100, '0xFIRST');
    expect(() =>
      anchor.recordAnchorSubmission('natoshisakamoto', 'l2', 100, '0xSECOND')
    ).toThrow(/already exists/);
  });

  it('same epoch allowed on different tiers', () => {
    anchor.recordAnchorSubmission('shindevlin', 'l2', 1000, '0xL2_TX');
    expect(() =>
      anchor.recordAnchorSubmission('shindevlin', 'ethereum', 1000, '0xETH_TX')
    ).not.toThrow();
  });

  it('throws if submitter is missing', () => {
    expect(() => anchor.recordAnchorSubmission('', 'l2', 100, '0xTX')).toThrow(/submitter/);
  });

  it('throws if txHash is missing', () => {
    expect(() => anchor.recordAnchorSubmission('shindevlin', 'l2', 100, '')).toThrow(/txHash/);
  });

  it('throws for invalid tier', () => {
    expect(() =>
      anchor.recordAnchorSubmission('shindevlin', 'polygon', 100, '0xTX')
    ).toThrow(/invalid tier/);
  });
});

describe('anchorSubmission — getAnchorHistory', () => {
  beforeEach(() => anchor.resetForTests());

  it('returns empty array when no anchors recorded', () => {
    const history = anchor.getAnchorHistory('l2');
    expect(history).toHaveLength(0);
  });

  it('returns recorded anchors newest-first', () => {
    anchor.recordAnchorSubmission('shindevlin', 'l2', 100, '0xTX100');
    anchor.recordAnchorSubmission('shindevlin', 'l2', 200, '0xTX200');
    const history = anchor.getAnchorHistory('l2');
    expect(history[0].epoch).toBe(200);
    expect(history[1].epoch).toBe(100);
  });

  it('respects limit parameter', () => {
    for (let i = 1; i <= 5; i++) {
      anchor.recordAnchorSubmission('shindevlin', 'l2', i * 100, '0xTX_' + i);
    }
    const history = anchor.getAnchorHistory('l2', 3);
    expect(history).toHaveLength(3);
  });

  it('tiers are isolated — l2 history does not include ethereum anchors', () => {
    anchor.recordAnchorSubmission('shindevlin', 'l2', 100, '0xL2TX');
    anchor.recordAnchorSubmission('shindevlin', 'ethereum', 1000, '0xETHTX');
    const l2History = anchor.getAnchorHistory('l2');
    expect(l2History).toHaveLength(1);
    expect(l2History[0].tier).toBe('l2');
  });
});

describe('anchorSubmission — estimateAnchorCost / getAnchorReward', () => {
  it('L2 anchor costs less than Ethereum anchor', () => {
    expect(anchor.estimateAnchorCost('l2')).toBeLessThan(anchor.estimateAnchorCost('ethereum'));
  });

  it('Bitcoin anchor costs the most (per anchor)', () => {
    expect(anchor.estimateAnchorCost('bitcoin')).toBeGreaterThan(anchor.estimateAnchorCost('ethereum'));
  });

  it('rewards scale with cost/rarity', () => {
    expect(anchor.getAnchorReward('bitcoin')).toBeGreaterThan(anchor.getAnchorReward('l2'));
  });

  it('all costs are positive numbers', () => {
    anchor.VALID_TIERS.forEach(tier => {
      expect(anchor.estimateAnchorCost(tier)).toBeGreaterThan(0);
      expect(anchor.getAnchorReward(tier)).toBeGreaterThan(0);
    });
  });

  it('throws for invalid tier', () => {
    expect(() => anchor.estimateAnchorCost('solana')).toThrow(/invalid tier/);
    expect(() => anchor.getAnchorReward('solana')).toThrow(/invalid tier/);
  });
});

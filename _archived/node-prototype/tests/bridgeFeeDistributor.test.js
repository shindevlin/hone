"use strict";

/**
 * Bridge Fee Distributor tests — v2.16-alpha
 *
 * Covers fee math (wrap + unwrap tiers), LP distribution by weight,
 * and smoothing buffer accrual.
 */

const dist = require('../src/services/bridgeFeeDistributor');
const bridge = require('../src/services/bridgeRegistry');

function setupChainWithFunders() {
  bridge.resetForTests();
  bridge.registerChain('base', {
    name: 'Base',
    bridge_reserve_address: '0xBASE',
  });
  // Fund with two LPs of different amounts and same lock period
  bridge.fundBridge('shindevlin', 'base', 2000, 365);
  bridge.fundBridge('natoshisakamoto', 'base', 1000, 365);
}

describe('bridgeFeeDistributor — constants', () => {
  it('WRAP_FEE_BPS is 5 (0.05%)', () => {
    expect(dist.WRAP_FEE_BPS).toBe(5);
  });

  it('UNWRAP_SMALL_BPS is 20 (0.20%)', () => {
    expect(dist.UNWRAP_SMALL_BPS).toBe(20);
  });

  it('UNWRAP_MID_BPS is 15 (0.15%)', () => {
    expect(dist.UNWRAP_MID_BPS).toBe(15);
  });

  it('UNWRAP_LARGE_BPS is 10 (0.10%)', () => {
    expect(dist.UNWRAP_LARGE_BPS).toBe(10);
  });

  it('SMOOTHING_FRACTION is between 0.10 and 0.20 (spec: 10-20%)', () => {
    expect(dist.SMOOTHING_FRACTION).toBeGreaterThanOrEqual(0.10);
    expect(dist.SMOOTHING_FRACTION).toBeLessThanOrEqual(0.20);
  });
});

describe('bridgeFeeDistributor — computeWrapFee', () => {
  it('computes flat 0.05% fee', () => {
    const result = dist.computeWrapFee(1000);
    expect(result.fee).toBeCloseTo(0.5, 5);
    expect(result.net).toBeCloseTo(999.5, 5);
    expect(result.rate_bps).toBe(5);
  });

  it('fee + net = gross (conservation)', () => {
    const result = dist.computeWrapFee(12345.67);
    expect(result.fee + result.net).toBeCloseTo(12345.67, 4);
  });

  it('same rate for large amounts (no tier change)', () => {
    const small = dist.computeWrapFee(10);
    const large = dist.computeWrapFee(1000000);
    expect(small.rate_bps).toBe(large.rate_bps);
  });

  it('throws for zero or negative amount', () => {
    expect(() => dist.computeWrapFee(0)).toThrow();
    expect(() => dist.computeWrapFee(-1)).toThrow();
  });

  it('throws for non-numeric amount', () => {
    expect(() => dist.computeWrapFee('abc')).toThrow();
    expect(() => dist.computeWrapFee(null)).toThrow();
  });
});

describe('bridgeFeeDistributor — computeUnwrapFee', () => {
  it('small tier: < 1,000 → 0.20%', () => {
    const result = dist.computeUnwrapFee(500);
    expect(result.tier).toBe('small');
    expect(result.rate_bps).toBe(20);
    expect(result.fee).toBeCloseTo(1.0, 5);
    expect(result.net).toBeCloseTo(499.0, 5);
  });

  it('boundary at exactly 1,000 → mid tier', () => {
    const result = dist.computeUnwrapFee(1000);
    expect(result.tier).toBe('mid');
    expect(result.rate_bps).toBe(15);
  });

  it('mid tier: 1,000-100,000 → 0.15%', () => {
    const result = dist.computeUnwrapFee(50000);
    expect(result.tier).toBe('mid');
    expect(result.fee).toBeCloseTo(75, 5);
  });

  it('boundary at exactly 100,000 → mid tier', () => {
    const result = dist.computeUnwrapFee(100000);
    expect(result.tier).toBe('mid');
  });

  it('large tier: > 100,000 → 0.10%', () => {
    const result = dist.computeUnwrapFee(200000);
    expect(result.tier).toBe('large');
    expect(result.rate_bps).toBe(10);
    expect(result.fee).toBeCloseTo(200, 5);
  });

  it('fee + net = gross (conservation)', () => {
    const result = dist.computeUnwrapFee(99999);
    expect(result.fee + result.net).toBeCloseTo(99999, 4);
  });

  it('throws for invalid amount', () => {
    expect(() => dist.computeUnwrapFee(0)).toThrow();
    expect(() => dist.computeUnwrapFee(-50)).toThrow();
  });
});

describe('bridgeFeeDistributor — distributeFeesToLPs', () => {
  beforeEach(setupChainWithFunders);
  afterEach(() => bridge.resetForTests());

  it('distributes fees proportionally to LP weight', () => {
    // shindevlin: 2000 HONE × ~365 days = higher weight
    // natoshisakamoto: 1000 HONE × ~365 days = lower weight (2:1 ratio)
    const distributions = dist.distributeFeesToLPs('base', 100, 'HONE', bridge);
    expect(distributions).toHaveLength(2);
    const shin = distributions.find(d => d.funder === 'shindevlin');
    const nato = distributions.find(d => d.funder === 'natoshisakamoto');
    expect(shin).toBeDefined();
    expect(nato).toBeDefined();
    // shindevlin should get ~2x natoshisakamoto (2000 vs 1000 HONE)
    expect(shin.amount).toBeGreaterThan(nato.amount);
    expect(shin.amount / nato.amount).toBeCloseTo(2, 0);
  });

  it('shares sum to total fee (within rounding)', () => {
    const distributions = dist.distributeFeesToLPs('base', 100, 'HONE', bridge);
    const total = distributions.reduce((s, d) => s + d.amount, 0);
    expect(total).toBeCloseTo(100, 4);
  });

  it('currency is passed through correctly', () => {
    const distributions = dist.distributeFeesToLPs('base', 50, 'wHONE', bridge);
    distributions.forEach(d => expect(d.currency).toBe('wHONE'));
  });

  it('returns empty array if no funders', () => {
    bridge.resetForTests();
    bridge.registerChain('base', { name: 'Base', bridge_reserve_address: '0xBASE' });
    const distributions = dist.distributeFeesToLPs('base', 100, 'HONE', bridge);
    expect(distributions).toHaveLength(0);
  });

  it('returns empty array if fee is zero', () => {
    const distributions = dist.distributeFeesToLPs('base', 0, 'HONE', bridge);
    expect(distributions).toHaveLength(0);
  });

  it('throws if bridgeRegistry is missing', () => {
    expect(() => dist.distributeFeesToLPs('base', 100, 'HONE', null)).toThrow(/bridgeRegistry required/);
  });
});

describe('bridgeFeeDistributor — accrueSmoothingBuffer', () => {
  beforeEach(setupChainWithFunders);
  afterEach(() => bridge.resetForTests());

  it('routes SMOOTHING_FRACTION of fees to buffer', () => {
    const result = dist.accrueSmoothingBuffer('base', 100, bridge);
    expect(result.buffered).toBeCloseTo(100 * dist.SMOOTHING_FRACTION, 5);
    expect(result.lp_distributable).toBeCloseTo(100 * (1 - dist.SMOOTHING_FRACTION), 5);
  });

  it('buffered + lp_distributable = fee (conservation)', () => {
    const result = dist.accrueSmoothingBuffer('base', 200, bridge);
    expect(result.buffered + result.lp_distributable).toBeCloseTo(200, 5);
  });

  it('returns zero amounts for zero fee', () => {
    const result = dist.accrueSmoothingBuffer('base', 0, bridge);
    expect(result.buffered).toBe(0);
    expect(result.lp_distributable).toBe(0);
  });

  it('actually increases bridge smoothing buffer balance', () => {
    const before = bridge.getSmoothingBuffer('base');
    dist.accrueSmoothingBuffer('base', 100, bridge);
    const after = bridge.getSmoothingBuffer('base');
    expect(after).toBeGreaterThan(before);
  });

  it('throws if bridgeRegistry is missing', () => {
    expect(() => dist.accrueSmoothingBuffer('base', 100, null)).toThrow(/bridgeRegistry required/);
  });
});

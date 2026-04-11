"use strict";

/**
 * Oracle Feeds tests (v2.13-delta)
 *
 * Covers registration, report submission, verifier allowlists, median
 * consensus, outlier detection, status transitions, and accessors.
 */

const oracle = require('../src/services/oracleFeeds');

function baseSpec(overrides) {
  return Object.assign({
    description: 'Bitcoin / US Dollar price',
    unit: 'USD',
    decimals: 8,
    update_interval_epochs: 12,
    min_verifiers: 3,
    max_deviation_bps: 100, // 1% tolerance
  }, overrides || {});
}

describe('oracleFeeds — feed ID parsing', () => {
  beforeEach(() => oracle.resetForTests());

  it('accepts <owner>/<name>', () => {
    const p = oracle.parseFeedId('alice/btc-usd');
    expect(p.ok).toBe(true);
    expect(p.owner).toBe('alice');
    expect(p.name).toBe('btc-usd');
  });

  it('rejects missing slash', () => {
    expect(oracle.parseFeedId('btcusd').ok).toBe(false);
  });

  it('rejects invalid chars in owner or name', () => {
    expect(oracle.parseFeedId('Alice/btc').ok).toBe(false);
    expect(oracle.parseFeedId('alice/BTC').ok).toBe(false);
    expect(oracle.parseFeedId('alice/_btc').ok).toBe(false);
  });

  it('rejects non-string', () => {
    expect(oracle.parseFeedId(null).ok).toBe(false);
    expect(oracle.parseFeedId(42).ok).toBe(false);
  });
});

describe('oracleFeeds — registration', () => {
  beforeEach(() => oracle.resetForTests());

  it('registers a new feed', () => {
    const rec = oracle.registerFeed('alice', 'alice/btc-usd', baseSpec(), { epoch: 100 });
    expect(rec.feed_id).toBe('alice/btc-usd');
    expect(rec.owner).toBe('alice');
    expect(rec.status).toBe('active');
    expect(rec.decimals).toBe(8);
    expect(rec.min_verifiers).toBe(3);
    expect(rec.max_deviation_bps).toBe(100);
    expect(rec.verifiers).toEqual([]);
    expect(rec.created_epoch).toBe(100);
    expect(rec.last_value).toBeNull();
  });

  it('rejects registration where feed_id prefix does not match owner', () => {
    expect(() => oracle.registerFeed('alice', 'bob/btc-usd', baseSpec())).toThrow(/owner prefix/);
  });

  it('rejects invalid decimals', () => {
    expect(() => oracle.registerFeed('alice', 'alice/btc-usd', baseSpec({ decimals: 19 }))).toThrow(/decimals/);
    expect(() => oracle.registerFeed('alice', 'alice/btc-usd', baseSpec({ decimals: -1 }))).toThrow(/decimals/);
  });

  it('rejects invalid min_verifiers', () => {
    expect(() => oracle.registerFeed('alice', 'alice/btc-usd', baseSpec({ min_verifiers: 0 }))).toThrow(/min_verifiers/);
  });

  it('rejects invalid max_deviation_bps', () => {
    expect(() => oracle.registerFeed('alice', 'alice/btc-usd', baseSpec({ max_deviation_bps: -1 }))).toThrow(/max_deviation_bps/);
  });

  it('rejects missing description / unit', () => {
    expect(() => oracle.registerFeed('alice', 'alice/btc-usd', baseSpec({ description: '' }))).toThrow(/description/);
    expect(() => oracle.registerFeed('alice', 'alice/btc-usd', baseSpec({ unit: '' }))).toThrow(/unit/);
  });

  it('accepts optional verifier allowlist', () => {
    const rec = oracle.registerFeed('alice', 'alice/btc-usd', baseSpec({ verifiers: ['bob', 'carol', 'dave'] }));
    expect(rec.verifiers).toEqual(['bob', 'carol', 'dave']);
  });

  it('deduplicates verifier list', () => {
    const rec = oracle.registerFeed('alice', 'alice/btc-usd', baseSpec({ verifiers: ['bob', 'bob', 'carol'] }));
    expect(rec.verifiers).toEqual(['bob', 'carol']);
  });

  it('updates existing feed when called again by the same owner', () => {
    oracle.registerFeed('alice', 'alice/btc-usd', baseSpec());
    const updated = oracle.registerFeed('alice', 'alice/btc-usd', baseSpec({ min_verifiers: 5 }), { epoch: 200 });
    expect(updated.min_verifiers).toBe(5);
    expect(updated.last_updated_epoch).toBe(200);
  });

  it('rejects updates from a different owner', () => {
    oracle.registerFeed('alice', 'alice/btc-usd', baseSpec());
    expect(() => oracle.registerFeed('bob', 'alice/btc-usd', baseSpec())).toThrow(/owner prefix/);
  });
});

describe('oracleFeeds — status transitions', () => {
  beforeEach(() => {
    oracle.resetForTests();
    oracle.registerFeed('alice', 'alice/btc-usd', baseSpec());
  });

  it('retires a feed (owner only)', () => {
    const rec = oracle.retireFeed('alice', 'alice/btc-usd', 50);
    expect(rec.status).toBe('retired');
    expect(rec.retired_epoch).toBe(50);
  });

  it('rejects retire from non-owner', () => {
    expect(() => oracle.retireFeed('bob', 'alice/btc-usd', 50)).toThrow(/only the owner/);
  });

  it('pauses and resumes', () => {
    const paused = oracle.pauseFeed('alice', 'alice/btc-usd', 10);
    expect(paused.status).toBe('paused');
    const resumed = oracle.resumeFeed('alice', 'alice/btc-usd', 11);
    expect(resumed.status).toBe('active');
  });

  it('cannot pause a retired feed', () => {
    oracle.retireFeed('alice', 'alice/btc-usd', 50);
    expect(() => oracle.pauseFeed('alice', 'alice/btc-usd', 60)).toThrow(/retired/);
  });

  it('cannot resume a non-paused feed', () => {
    expect(() => oracle.resumeFeed('alice', 'alice/btc-usd', 5)).toThrow(/not paused/);
  });
});

describe('oracleFeeds — report submission', () => {
  beforeEach(() => {
    oracle.resetForTests();
    oracle.registerFeed('alice', 'alice/btc-usd', baseSpec());
  });

  it('accepts a report from any verifier when allowlist is empty', () => {
    const r = oracle.submitReport('randomnode', 'alice/btc-usd', 100000, 5);
    expect(r.verifier).toBe('randomnode');
    expect(r.value).toBe(100000);
    expect(r.epoch).toBe(5);
  });

  it('rejects a report from a verifier not in the allowlist', () => {
    oracle.registerFeed('alice', 'alice/btc-usd', baseSpec({ verifiers: ['bob', 'carol'] }));
    expect(() => oracle.submitReport('stranger', 'alice/btc-usd', 100000, 5)).toThrow(/allowlist/);
  });

  it('rejects duplicate reports from same verifier for same epoch', () => {
    oracle.submitReport('bob', 'alice/btc-usd', 100000, 5);
    expect(() => oracle.submitReport('bob', 'alice/btc-usd', 101000, 5)).toThrow(/already reported/);
  });

  it('allows a verifier to report different epochs', () => {
    oracle.submitReport('bob', 'alice/btc-usd', 100000, 5);
    const r = oracle.submitReport('bob', 'alice/btc-usd', 101000, 6);
    expect(r.epoch).toBe(6);
  });

  it('rejects non-finite values', () => {
    expect(() => oracle.submitReport('bob', 'alice/btc-usd', Infinity, 5)).toThrow(/finite/);
    expect(() => oracle.submitReport('bob', 'alice/btc-usd', NaN, 5)).toThrow(/finite/);
    expect(() => oracle.submitReport('bob', 'alice/btc-usd', 'not-a-number', 5)).toThrow(/finite/);
  });

  it('rejects reports on paused feeds', () => {
    oracle.pauseFeed('alice', 'alice/btc-usd', 1);
    expect(() => oracle.submitReport('bob', 'alice/btc-usd', 100000, 5)).toThrow(/not active/);
  });

  it('rejects reports on retired feeds', () => {
    oracle.retireFeed('alice', 'alice/btc-usd', 1);
    expect(() => oracle.submitReport('bob', 'alice/btc-usd', 100000, 5)).toThrow(/not active/);
  });

  it('rejects reports for unknown feeds', () => {
    expect(() => oracle.submitReport('bob', 'alice/ghost', 100, 5)).toThrow(/not found/);
  });

  it('rejects reports after finalization', () => {
    oracle.submitReport('bob', 'alice/btc-usd', 100000, 5);
    oracle.submitReport('carol', 'alice/btc-usd', 100100, 5);
    oracle.submitReport('dave', 'alice/btc-usd', 99900, 5);
    oracle.finalizeFeedEpoch('alice/btc-usd', 5);
    expect(() => oracle.submitReport('eve', 'alice/btc-usd', 100000, 5)).toThrow(/already finalized/);
  });
});

describe('oracleFeeds — median computation', () => {
  beforeEach(() => {
    oracle.resetForTests();
    oracle.registerFeed('alice', 'alice/btc-usd', baseSpec());
  });

  it('median of odd count', () => {
    expect(oracle._median([1, 2, 3])).toBe(2);
    expect(oracle._median([5, 1, 3])).toBe(3);
    expect(oracle._median([100])).toBe(100);
  });

  it('median of even count averages the middle two', () => {
    expect(oracle._median([1, 2, 3, 4])).toBe(2.5);
    expect(oracle._median([10, 20])).toBe(15);
  });

  it('median is robust to a single outlier', () => {
    expect(oracle._median([100, 100, 100, 100, 999999])).toBe(100);
  });

  it('returns null for empty input', () => {
    expect(oracle._median([])).toBeNull();
  });
});

describe('oracleFeeds — finalization & consensus', () => {
  beforeEach(() => {
    oracle.resetForTests();
    oracle.registerFeed('alice', 'alice/btc-usd', baseSpec({ max_deviation_bps: 100 }));
  });

  it('computes median and updates last_value', () => {
    oracle.submitReport('v1', 'alice/btc-usd', 100000, 10);
    oracle.submitReport('v2', 'alice/btc-usd', 100100, 10);
    oracle.submitReport('v3', 'alice/btc-usd', 99900, 10);

    const fin = oracle.finalizeFeedEpoch('alice/btc-usd', 10);
    expect(fin.median).toBe(100000);
    expect(fin.report_count).toBe(3);
    expect(fin.outliers).toHaveLength(0);
    expect(fin.insiders).toEqual(expect.arrayContaining(['v1', 'v2', 'v3']));

    expect(oracle.getFeedValue('alice/btc-usd')).toBe(100000);
  });

  it('flags outliers beyond max_deviation_bps', () => {
    // median will be 100000, max deviation is 100 bps = 1%
    // 100000 ± 1% = 99000..101000
    oracle.submitReport('honest1', 'alice/btc-usd', 100000, 10);
    oracle.submitReport('honest2', 'alice/btc-usd', 100050, 10);
    oracle.submitReport('honest3', 'alice/btc-usd', 99950, 10);
    oracle.submitReport('liar', 'alice/btc-usd', 200000, 10); // 100% off → outlier

    const fin = oracle.finalizeFeedEpoch('alice/btc-usd', 10);
    expect(fin.outliers).toHaveLength(1);
    expect(fin.outliers[0].verifier).toBe('liar');
    expect(fin.insiders).toEqual(expect.arrayContaining(['honest1', 'honest2', 'honest3']));
  });

  it('rejects finalization when min_verifiers not met', () => {
    oracle.submitReport('v1', 'alice/btc-usd', 100000, 10);
    oracle.submitReport('v2', 'alice/btc-usd', 100100, 10);
    // min_verifiers = 3, only 2 reports
    expect(() => oracle.finalizeFeedEpoch('alice/btc-usd', 10)).toThrow(/insufficient/);
  });

  it('is idempotent (second call returns same finalization)', () => {
    oracle.submitReport('v1', 'alice/btc-usd', 100, 10);
    oracle.submitReport('v2', 'alice/btc-usd', 100, 10);
    oracle.submitReport('v3', 'alice/btc-usd', 100, 10);

    const first = oracle.finalizeFeedEpoch('alice/btc-usd', 10);
    const second = oracle.finalizeFeedEpoch('alice/btc-usd', 10);
    expect(second).toBe(first);
  });

  it('updates verifier stats including outlier count', () => {
    oracle.registerFeed('alice', 'alice/weather', baseSpec({ max_deviation_bps: 500 }));
    oracle.submitReport('v1', 'alice/weather', 20, 1);
    oracle.submitReport('v2', 'alice/weather', 21, 1);
    oracle.submitReport('v3', 'alice/weather', 100, 1); // far outlier
    oracle.finalizeFeedEpoch('alice/weather', 1);

    const v1Stats = oracle.getVerifierStats('v1');
    expect(v1Stats.total_reports).toBe(1);
    expect(v1Stats.total_outliers).toBe(0);

    const v3Stats = oracle.getVerifierStats('v3');
    expect(v3Stats.total_reports).toBe(1);
    expect(v3Stats.total_outliers).toBe(1);
    expect(v3Stats.outlier_rate).toBe(1);
  });

  it('handles zero-value medians correctly', () => {
    oracle.registerFeed('alice', 'alice/zero-feed', baseSpec({ max_deviation_bps: 0 }));
    oracle.submitReport('v1', 'alice/zero-feed', 0, 1);
    oracle.submitReport('v2', 'alice/zero-feed', 0, 1);
    oracle.submitReport('v3', 'alice/zero-feed', 1, 1);

    const fin = oracle.finalizeFeedEpoch('alice/zero-feed', 1);
    expect(fin.median).toBe(0);
    expect(fin.outliers).toHaveLength(1);
    expect(fin.outliers[0].verifier).toBe('v3');
  });
});

describe('oracleFeeds — accessors', () => {
  beforeEach(() => {
    oracle.resetForTests();
    oracle.registerFeed('alice', 'alice/btc-usd', baseSpec());
    oracle.registerFeed('bob', 'bob/weather-nyc', baseSpec({
      description: 'NYC temperature',
      unit: 'celsius',
      decimals: 2,
    }));
  });

  it('getAllFeeds returns all', () => {
    const all = oracle.getAllFeeds();
    expect(all).toHaveLength(2);
  });

  it('getAllFeeds filters by owner', () => {
    const alicesFeeds = oracle.getAllFeeds({ owner: 'alice' });
    expect(alicesFeeds).toHaveLength(1);
    expect(alicesFeeds[0].owner).toBe('alice');
  });

  it('getAllFeeds filters by status', () => {
    oracle.retireFeed('bob', 'bob/weather-nyc', 5);
    const active = oracle.getAllFeeds({ status: 'active' });
    expect(active).toHaveLength(1);
    expect(active[0].feed_id).toBe('alice/btc-usd');
  });

  it('getReportsForEpoch returns submitted reports', () => {
    oracle.submitReport('v1', 'alice/btc-usd', 100, 10);
    oracle.submitReport('v2', 'alice/btc-usd', 101, 10);
    const reports = oracle.getReportsForEpoch('alice/btc-usd', 10);
    expect(reports).toHaveLength(2);
  });

  it('getFinalization returns null before finalize', () => {
    expect(oracle.getFinalization('alice/btc-usd', 10)).toBeNull();
  });

  it('getFeedValue returns null before first finalization', () => {
    expect(oracle.getFeedValue('alice/btc-usd')).toBeNull();
  });
});

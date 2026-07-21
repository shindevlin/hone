"use strict";

/**
 * Mempool fee market tests — v2.12-beta
 */

const feeMarket = require('../src/p2p/mempoolFeeMarket');

function makeTx(opts) {
  opts = opts || {};
  return {
    type: 'TRANSFER',
    from: opts.from || 'alice',
    to: opts.to || 'bob',
    amount: opts.amount || 1,
    token: 'HONE',
    nonce: opts.nonce || Date.now() + Math.random(),
    timestamp: opts.timestamp || Date.now(),
    fee: opts.fee || 0,
    memo: opts.memo || null,
  };
}

describe('mempoolFeeMarket (v2.12-beta)', () => {
  describe('feePerByte', () => {
    test('null tx returns 0', () => {
      expect(feeMarket.feePerByte(null)).toBe(0);
    });

    test('zero fee returns 0', () => {
      expect(feeMarket.feePerByte(makeTx({ fee: 0 }))).toBe(0);
    });

    test('missing fee returns 0', () => {
      const tx = makeTx({});
      delete tx.fee;
      expect(feeMarket.feePerByte(tx)).toBe(0);
    });

    test('positive fee returns positive value', () => {
      const tx = makeTx({ fee: 1 });
      expect(feeMarket.feePerByte(tx)).toBeGreaterThan(0);
    });

    test('higher fee on same-size tx → higher fee_per_byte', () => {
      const lo = makeTx({ fee: 1 });
      const hi = makeTx({ fee: 10 });
      expect(feeMarket.feePerByte(hi)).toBeGreaterThan(feeMarket.feePerByte(lo));
    });
  });

  describe('sortByFee', () => {
    test('empty list returns empty', () => {
      expect(feeMarket.sortByFee([])).toEqual([]);
    });

    test('single tx returns itself', () => {
      const tx = makeTx({ fee: 1 });
      expect(feeMarket.sortByFee([tx])).toEqual([tx]);
    });

    test('descending fee order', () => {
      const lo = makeTx({ fee: 1, nonce: 1 });
      const mid = makeTx({ fee: 5, nonce: 2 });
      const hi = makeTx({ fee: 100, nonce: 3 });
      const sorted = feeMarket.sortByFee([lo, mid, hi]);
      expect(sorted[0]).toBe(hi);
      expect(sorted[1]).toBe(mid);
      expect(sorted[2]).toBe(lo);
    });

    test('equal fee_per_byte preserves arrival order (stable)', () => {
      // Use identical-shape transactions (only nonce + tag differ)
      // so fee_per_byte is identical and the sort is deterministic
      // by arrival order alone.
      const sameMemo = 'tag';
      const a = makeTx({ fee: 5, nonce: 100001, memo: sameMemo, from: 'aaa', to: 'bbb' });
      const b = makeTx({ fee: 5, nonce: 100002, memo: sameMemo, from: 'aaa', to: 'bbb' });
      const c = makeTx({ fee: 5, nonce: 100003, memo: sameMemo, from: 'aaa', to: 'bbb' });
      // Sanity: all three have the same fee_per_byte (within rounding)
      expect(feeMarket.feePerByte(a)).toBeCloseTo(feeMarket.feePerByte(b), 10);
      const sorted = feeMarket.sortByFee([a, b, c]);
      expect(sorted.map((t) => t.nonce)).toEqual([100001, 100002, 100003]);
    });

    test('free txs sort after fee-paying ones', () => {
      const free = makeTx({ fee: 0, nonce: 1, memo: 'free' });
      const paid = makeTx({ fee: 10, nonce: 2, memo: 'paid' });
      const sorted = feeMarket.sortByFee([free, paid]);
      expect(sorted[0].memo).toBe('paid');
      expect(sorted[1].memo).toBe('free');
    });
  });

  describe('selectTransactionsForBlock', () => {
    test('empty mempool returns empty selection', () => {
      const result = feeMarket.selectTransactionsForBlock([]);
      expect(result.selected).toEqual([]);
      expect(result.total_fees_collected).toBe(0);
    });

    test('all txs fit when block is empty', () => {
      const txs = [makeTx({ fee: 1, nonce: 1 }), makeTx({ fee: 2, nonce: 2 })];
      const result = feeMarket.selectTransactionsForBlock(txs);
      expect(result.selected.length).toBe(2);
      expect(result.skipped_no_space.length).toBe(0);
    });

    test('high-fee txs selected first', () => {
      const lo = makeTx({ fee: 1, nonce: 1, memo: 'low' });
      const hi = makeTx({ fee: 100, nonce: 2, memo: 'high' });
      const result = feeMarket.selectTransactionsForBlock([lo, hi]);
      expect(result.selected[0].memo).toBe('high');
      expect(result.selected[1].memo).toBe('low');
    });

    test('min_fee_per_byte filter rejects low-fee txs', () => {
      const lo = makeTx({ fee: 1, nonce: 1, memo: 'low' });
      const hi = makeTx({ fee: 1000, nonce: 2, memo: 'high' });
      const result = feeMarket.selectTransactionsForBlock([lo, hi], {
        min_fee_per_byte: feeMarket.feePerByte(hi) * 0.5, // floor between lo and hi
      });
      expect(result.selected.length).toBe(1);
      expect(result.selected[0].memo).toBe('high');
      expect(result.skipped_low_fee.length).toBe(1);
      expect(result.skipped_low_fee[0].memo).toBe('low');
    });

    test('total_fees_collected sums included tx fees', () => {
      const txs = [
        makeTx({ fee: 5, nonce: 1 }),
        makeTx({ fee: 10, nonce: 2 }),
        makeTx({ fee: 3, nonce: 3 }),
      ];
      const result = feeMarket.selectTransactionsForBlock(txs);
      expect(result.total_fees_collected).toBe(18);
    });

    test('free txs included when block has space', () => {
      const free = makeTx({ fee: 0, nonce: 1, memo: 'free' });
      const paid = makeTx({ fee: 10, nonce: 2, memo: 'paid' });
      const result = feeMarket.selectTransactionsForBlock([free, paid]);
      expect(result.selected.length).toBe(2);
      // Paid first (sorted), free second
      expect(result.selected[0].memo).toBe('paid');
      expect(result.selected[1].memo).toBe('free');
    });

    test('respects block size cap (large mempool)', () => {
      // Generate many large txs that won't all fit
      const txs = [];
      for (let i = 0; i < 20000; i++) {
        txs.push(
          makeTx({
            from: 'alice-' + i,
            to: 'bob-' + i,
            amount: i,
            fee: i % 100, // varying fees
            nonce: i,
            memo: 'transfer ' + i,
          })
        );
      }
      const result = feeMarket.selectTransactionsForBlock(txs);
      expect(result.selected.length).toBeLessThan(txs.length);
      expect(result.skipped_no_space.length).toBeGreaterThan(0);
      expect(result.final_payload_bytes).toBeLessThanOrEqual(1024 * 1024);

      // Sanity: average fee_per_byte of selected should be >= average
      // fee_per_byte of skipped (the whole point of fee-priority sort)
      function avgFpb(list) {
        if (list.length === 0) return 0;
        let sum = 0;
        for (const t of list) sum += feeMarket.feePerByte(t);
        return sum / list.length;
      }
      expect(avgFpb(result.selected)).toBeGreaterThanOrEqual(avgFpb(result.skipped_no_space));
    });
  });

  describe('congestionLevel', () => {
    test('empty mempool → 0', () => {
      expect(feeMarket.congestionLevel([])).toBe(0);
    });

    test('small mempool → low congestion', () => {
      const txs = [makeTx({ fee: 1 })];
      expect(feeMarket.congestionLevel(txs)).toBeLessThan(0.01);
    });

    test('large mempool → high congestion', () => {
      const txs = [];
      for (let i = 0; i < 5000; i++) {
        txs.push(
          makeTx({
            from: 'alice-' + i,
            to: 'bob-' + i,
            nonce: i,
            memo: 'transfer with a long memo string',
          })
        );
      }
      expect(feeMarket.congestionLevel(txs)).toBeGreaterThan(0.5);
    });
  });

  describe('suggestMinFeePerByte', () => {
    test('low congestion → floor of 0', () => {
      const txs = [makeTx({ fee: 0 })];
      expect(feeMarket.suggestMinFeePerByte(txs)).toBe(0);
    });

    test('high congestion → positive floor', () => {
      const txs = [];
      for (let i = 0; i < 5000; i++) {
        txs.push(
          makeTx({
            from: 'alice-' + i,
            to: 'bob-' + i,
            nonce: i,
            memo: 'transfer with a long memo string',
          })
        );
      }
      expect(feeMarket.suggestMinFeePerByte(txs)).toBeGreaterThan(0);
    });
  });
});

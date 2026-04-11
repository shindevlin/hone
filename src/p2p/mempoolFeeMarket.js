"use strict";

/**
 * BTCPC Mempool Fee Market — v2.12-beta
 * Shin Devlin
 *
 * Fee-aware transaction selection for block inclusion. Sibling to
 * src/p2p/mempool.js — does not modify the mempool itself, just adds
 * a smarter selector that miners use when deciding which pending
 * transactions to flush into the next block.
 *
 * Selection rules:
 *
 *  1. Compute fee_per_byte for each pending transaction.
 *     A transaction with no `fee` field has fee_per_byte = 0 (free tx).
 *
 *  2. Sort transactions descending by fee_per_byte.
 *
 *  3. Greedy pack into available block space, using
 *     src/chain/blockSizeCap.MAX_BLOCK_PAYLOAD_BYTES as the cap.
 *
 *  4. Free transactions (fee_per_byte = 0) still get included if
 *     there's space remaining after all fee-paying ones — BTCPC is
 *     not "fees only" by default, free txs work as long as the
 *     mempool isn't congested.
 *
 *  5. The miner can pass a `min_fee_per_byte` floor to reject low-fee
 *     transactions during congestion. Default is 0 (accept all).
 *
 * Fee distribution (when implemented in the miner):
 *  - Fees collected from a block go to the block's clock node (as
 *    a separate ledger entry from the block reward), routed via
 *    the same path as recordMiningReward
 *  - The fee market is INCLUSION priority only. The chain's reward
 *    structure is unchanged — miners still earn block rewards from
 *    the emission schedule, fees are a bonus on top.
 *
 * v2.12-beta only ships the selection logic. Wiring it into the
 * miner's actual block-flush path lands as a separate commit so
 * each piece can be tested independently.
 */

var blockSizeCap = require("../chain/blockSizeCap");

/**
 * Compute fee_per_byte for a transaction.
 * fee is in BTCPC; size is the JSON byte length.
 *
 * Free transactions (no fee field, or fee = 0) return 0.
 */
function feePerByte(tx) {
  if (!tx) return 0;
  var fee = Number(tx.fee) || 0;
  if (fee <= 0) return 0;
  var size = Buffer.byteLength(JSON.stringify(tx), "utf8");
  if (size === 0) return 0;
  return fee / size;
}

/**
 * Sort transactions descending by fee_per_byte. Stable sort:
 * transactions with equal fee_per_byte preserve their original
 * order (which is arrival time from the mempool). Free txs (fee=0)
 * end up last but in arrival order.
 */
function sortByFee(transactions) {
  // Decorate with original index for stable sort
  var decorated = transactions.map(function (tx, i) {
    return { tx: tx, fee_per_byte: feePerByte(tx), originalIndex: i };
  });
  decorated.sort(function (a, b) {
    if (b.fee_per_byte !== a.fee_per_byte) {
      return b.fee_per_byte - a.fee_per_byte;
    }
    // Equal fee_per_byte → preserve arrival order
    return a.originalIndex - b.originalIndex;
  });
  return decorated.map(function (d) { return d.tx; });
}

/**
 * Select transactions for inclusion in the next block.
 *
 * @param {object[]} pendingTxs — list from mempool.getTransactions()
 * @param {object} [options]
 * @param {number} [options.min_fee_per_byte] — reject txs below this floor
 * @param {object} [options.basePayload] — base block payload (without
 *        ledger_entries) used to compute available space accurately
 * @returns {{
 *   selected: object[],     // transactions to include in this block
 *   skipped_low_fee: object[], // dropped because below min_fee_per_byte
 *   skipped_no_space: object[], // dropped because block is full
 *   total_fees_collected: number,
 *   final_payload_bytes: number,
 * }}
 */
function selectTransactionsForBlock(pendingTxs, options) {
  options = options || {};
  var minFeePerByte = Number.isFinite(options.min_fee_per_byte)
    ? options.min_fee_per_byte
    : 0;
  var basePayload = options.basePayload || { ledger_entries: [] };

  if (!Array.isArray(pendingTxs) || pendingTxs.length === 0) {
    return {
      selected: [],
      skipped_low_fee: [],
      skipped_no_space: [],
      total_fees_collected: 0,
      final_payload_bytes: blockSizeCap.estimateBlockPayloadSize(basePayload),
    };
  }

  // Filter low-fee first
  var skippedLowFee = [];
  var candidates = [];
  for (var i = 0; i < pendingTxs.length; i++) {
    var tx = pendingTxs[i];
    if (feePerByte(tx) < minFeePerByte) {
      skippedLowFee.push(tx);
    } else {
      candidates.push(tx);
    }
  }

  // Sort by fee_per_byte descending
  var sorted = sortByFee(candidates);

  // Greedy pack into block space using blockSizeCap.trimEntriesToCap.
  // The trim helper is order-preserving, which is what we want now
  // that we've sorted by fee priority.
  var trimResult = blockSizeCap.trimEntriesToCap(sorted, basePayload);

  // Sum fees collected
  var totalFees = 0;
  for (var j = 0; j < trimResult.kept.length; j++) {
    totalFees += Number(trimResult.kept[j].fee) || 0;
  }

  return {
    selected: trimResult.kept,
    skipped_low_fee: skippedLowFee,
    skipped_no_space: trimResult.dropped,
    total_fees_collected: _round(totalFees),
    final_payload_bytes: trimResult.final_bytes,
  };
}

/**
 * Compute mempool congestion level (0.0 to 1.0).
 *
 * This is just total bytes of pending transactions divided by the
 * block size cap. A miner can use this to set a min_fee_per_byte
 * dynamically: at low congestion accept everything, at high
 * congestion only accept higher-fee txs.
 *
 * Returns 0.0 when mempool is empty, 1.0 when pending bytes equal
 * one full block, > 1.0 when there are multiple blocks worth queued.
 */
function congestionLevel(pendingTxs) {
  if (!Array.isArray(pendingTxs) || pendingTxs.length === 0) return 0;
  var totalBytes = 0;
  for (var i = 0; i < pendingTxs.length; i++) {
    totalBytes += Buffer.byteLength(JSON.stringify(pendingTxs[i]), "utf8");
  }
  return _round(totalBytes / blockSizeCap.MAX_BLOCK_PAYLOAD_BYTES);
}

/**
 * Suggest a min_fee_per_byte floor based on current congestion.
 *
 *   congestion < 0.5  → floor = 0 (accept everything, including free txs)
 *   congestion 0.5–1  → floor = base_fee × congestion
 *   congestion > 1    → floor = base_fee × congestion (multiplier scales)
 *
 * The base_fee is a chain parameter (default 0.0001 BTCPC per byte).
 */
function suggestMinFeePerByte(pendingTxs, opts) {
  opts = opts || {};
  var baseFee = Number.isFinite(opts.base_fee) ? opts.base_fee : 0.0001;
  var congestion = congestionLevel(pendingTxs);
  if (congestion < 0.5) return 0;
  return _round(baseFee * congestion);
}

function _round(n) {
  return parseFloat(Number(n).toFixed(10));
}

module.exports = {
  feePerByte: feePerByte,
  sortByFee: sortByFee,
  selectTransactionsForBlock: selectTransactionsForBlock,
  congestionLevel: congestionLevel,
  suggestMinFeePerByte: suggestMinFeePerByte,
};

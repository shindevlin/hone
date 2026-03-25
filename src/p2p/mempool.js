"use strict";

/**
 * BTCPC Transaction Mempool
 * Shin Devlin
 *
 * In-memory transaction pool for pending transactions awaiting
 * inclusion in an epoch/block. Size-limited to prevent memory exhaustion.
 */

const MAX_MEMPOOL_SIZE = 1000;

// Map<txId, transaction>
const pending = new Map();

// ---------------------------------------------------------------------------
// Transaction management
// ---------------------------------------------------------------------------

/**
 * Validate and add a transaction to the mempool.
 * Returns true if added, false if rejected (duplicate, full, invalid).
 */
function addTransaction(tx) {
  if (!tx) return false;

  // Must have a transaction ID
  const txId = tx.id || tx._id || tx.txId;
  if (!txId) return false;

  // Reject duplicates
  if (pending.has(txId)) return false;

  // Enforce size limit
  if (pending.size >= MAX_MEMPOOL_SIZE) {
    console.log("[BTCPC Mempool] Full (" + MAX_MEMPOOL_SIZE + " transactions), rejecting " + txId);
    return false;
  }

  // Basic validation
  if (!tx.type) return false;
  if (tx.type === "transfer" && (!tx.from || !tx.to || !tx.amount)) return false;
  if (tx.amount !== undefined && (typeof tx.amount !== "number" || tx.amount <= 0)) return false;

  // Store with receipt timestamp
  tx._mempoolAddedAt = Date.now();
  pending.set(txId, tx);

  return true;
}

/**
 * Get pending transactions for block inclusion.
 * Returns up to `limit` transactions, ordered by arrival time.
 */
function getTransactions(limit) {
  const txs = [];
  const max = limit || MAX_MEMPOOL_SIZE;

  for (const [, tx] of pending) {
    txs.push(tx);
    if (txs.length >= max) break;
  }

  return txs;
}

/**
 * Remove a transaction from the mempool (after inclusion in a block).
 */
function removeTransaction(txId) {
  return pending.delete(txId);
}

/**
 * Remove multiple transactions by their IDs.
 */
function removeTransactions(txIds) {
  let removed = 0;
  for (const id of txIds) {
    if (pending.delete(id)) removed++;
  }
  return removed;
}

/**
 * Check if a transaction exists in the mempool.
 */
function hasTransaction(txId) {
  return pending.has(txId);
}

/**
 * Get the current mempool size.
 */
function size() {
  return pending.size;
}

/**
 * Clear all pending transactions.
 */
function clear() {
  pending.clear();
}

/**
 * Get mempool stats.
 */
function getStats() {
  let oldestAge = 0;
  const now = Date.now();

  for (const [, tx] of pending) {
    if (tx._mempoolAddedAt) {
      const age = now - tx._mempoolAddedAt;
      if (age > oldestAge) oldestAge = age;
    }
  }

  return {
    size: pending.size,
    maxSize: MAX_MEMPOOL_SIZE,
    oldestAgeSec: Math.floor(oldestAge / 1000)
  };
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  addTransaction,
  getTransactions,
  removeTransaction,
  removeTransactions,
  hasTransaction,
  size,
  clear,
  getStats,
  MAX_MEMPOOL_SIZE
};

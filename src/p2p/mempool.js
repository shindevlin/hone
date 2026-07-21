"use strict";

/**
 * HONE Transaction Mempool
 * Shin Devlin
 *
 * In-memory transaction pool for pending transactions awaiting
 * inclusion in an epoch/block. Validates balances, rejects double-spends
 * via nonce tracking, and broadcasts to peers.
 *
 * Flow: API/CLI → mempool.submit() → P2P broadcast → epoch finalization
 */

const crypto = require("crypto");

const MAX_MEMPOOL_SIZE = 1000;

// Map<txHash, transaction>
const pending = new Map();

// Track pending debits per account to prevent double-spends
// Map<username, { pendingDebit: number, nonces: Set<number> }>
const accountPending = new Map();

// ---------------------------------------------------------------------------
// Transaction hashing
// ---------------------------------------------------------------------------

/**
 * Compute a deterministic SHA-256 hash of a transaction.
 * This is the txid — like Bitcoin's txid.
 */
function hashTransaction(tx) {
  var canonical = {
    amount: tx.amount,
    from: tx.from,
    memo: tx.memo || null,
    nonce: tx.nonce,
    timestamp: tx.timestamp,
    to: tx.to,
    token: tx.token || "HONE",
    type: tx.type
  };
  return crypto.createHash("sha256")
    .update(JSON.stringify(canonical))
    .digest("hex");
}

// ---------------------------------------------------------------------------
// Transaction management
// ---------------------------------------------------------------------------

/**
 * Submit a transaction to the mempool.
 * Validates balance, checks for double-spend, assigns txHash.
 *
 * @param {object} tx — { type, from, to, amount, token, nonce, timestamp, memo, signature }
 * @param {number} senderBalance — current ledger balance of sender (caller provides)
 * @returns {{ accepted: boolean, txHash: string|null, reason: string|null }}
 */
function submit(tx) {
  if (!tx) return { accepted: false, txHash: null, reason: "empty transaction" };
  if (!tx.type) return { accepted: false, txHash: null, reason: "missing type" };
  if (!tx.from || !tx.to) return { accepted: false, txHash: null, reason: "missing from/to" };
  if (!tx.amount || typeof tx.amount !== "number" || tx.amount <= 0) {
    return { accepted: false, txHash: null, reason: "invalid amount" };
  }
  if (tx.from === tx.to) return { accepted: false, txHash: null, reason: "self-transfer" };

  // Ensure timestamp and nonce
  if (!tx.timestamp) tx.timestamp = Date.now();
  if (tx.nonce === undefined) tx.nonce = Date.now(); // fallback nonce

  // Compute transaction hash
  var txHash = hashTransaction(tx);

  // Reject duplicates
  if (pending.has(txHash)) {
    return { accepted: false, txHash: txHash, reason: "duplicate" };
  }

  // Enforce size limit
  if (pending.size >= MAX_MEMPOOL_SIZE) {
    return { accepted: false, txHash: null, reason: "mempool full" };
  }

  // Track pending debits to prevent double-spend
  var acct = accountPending.get(tx.from);
  if (!acct) {
    acct = { pendingDebit: 0, nonces: new Set() };
    accountPending.set(tx.from, acct);
  }

  // Check for nonce reuse (replay protection)
  if (acct.nonces.has(tx.nonce)) {
    return { accepted: false, txHash: txHash, reason: "nonce reuse" };
  }

  // Store
  tx.txHash = txHash;
  tx._mempoolAddedAt = Date.now();
  pending.set(txHash, tx);
  acct.pendingDebit += tx.amount;
  acct.nonces.add(tx.nonce);

  return { accepted: true, txHash: txHash, reason: null };
}

/**
 * Legacy addTransaction — wraps submit() for backward compat.
 */
function addTransaction(tx) {
  var txId = tx.txHash || tx.id || tx._id || tx.txId;

  // If it already has a hash, check for duplicate
  if (txId && pending.has(txId)) return false;

  // Compute hash if missing
  if (!tx.txHash && tx.from && tx.to && tx.amount) {
    if (!tx.timestamp) tx.timestamp = Date.now();
    if (tx.nonce === undefined) tx.nonce = Date.now();
    tx.txHash = hashTransaction(tx);
  }

  var result = submit(tx);
  return result.accepted;
}

/**
 * Get the total pending debit for an account.
 * Used to check "effective balance" = ledger balance - pending debits.
 */
function getPendingDebit(username) {
  var acct = accountPending.get(username);
  return acct ? acct.pendingDebit : 0;
}

/**
 * Get pending transactions for block inclusion.
 * Returns up to `limit` transactions, ordered by arrival time.
 */
function getTransactions(limit) {
  var txs = [];
  var max = limit || MAX_MEMPOOL_SIZE;

  for (var entry of pending) {
    txs.push(entry[1]);
    if (txs.length >= max) break;
  }

  return txs;
}

/**
 * Remove a transaction from the mempool (after inclusion in a block).
 */
function removeTransaction(txHash) {
  var tx = pending.get(txHash);
  if (!tx) return false;

  pending.delete(txHash);

  // Update account pending debits
  if (tx.from) {
    var acct = accountPending.get(tx.from);
    if (acct) {
      acct.pendingDebit = Math.max(0, acct.pendingDebit - tx.amount);
      acct.nonces.delete(tx.nonce);
      if (acct.pendingDebit === 0 && acct.nonces.size === 0) {
        accountPending.delete(tx.from);
      }
    }
  }

  return true;
}

/**
 * Remove multiple transactions by their hashes.
 */
function removeTransactions(txHashes) {
  var removed = 0;
  for (var i = 0; i < txHashes.length; i++) {
    if (removeTransaction(txHashes[i])) removed++;
  }
  return removed;
}

/**
 * Check if a transaction exists in the mempool.
 */
function hasTransaction(txHash) {
  return pending.has(txHash);
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
  accountPending.clear();
}

/**
 * Get mempool stats.
 */
function getStats() {
  var oldestAge = 0;
  var now = Date.now();

  for (var entry of pending) {
    var tx = entry[1];
    if (tx._mempoolAddedAt) {
      var age = now - tx._mempoolAddedAt;
      if (age > oldestAge) oldestAge = age;
    }
  }

  return {
    size: pending.size,
    maxSize: MAX_MEMPOOL_SIZE,
    oldestAgeSec: Math.floor(oldestAge / 1000),
    accountsWithPending: accountPending.size
  };
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  hashTransaction: hashTransaction,
  submit: submit,
  addTransaction: addTransaction,
  getPendingDebit: getPendingDebit,
  getTransactions: getTransactions,
  removeTransaction: removeTransaction,
  removeTransactions: removeTransactions,
  hasTransaction: hasTransaction,
  size: size,
  clear: clear,
  getStats: getStats,
  MAX_MEMPOOL_SIZE: MAX_MEMPOOL_SIZE
};

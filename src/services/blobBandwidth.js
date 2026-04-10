"use strict";

/**
 * BTCPC-FS Bandwidth Accumulator — v2.11.1
 * Shin Devlin
 *
 * In-memory per-CID byte counter for storage hosts. Every time the
 * blob routes serve a byte via GET /api/blobs/:cid, the accumulator
 * ticks up. Periodically, the host flushes accumulated counts to chain
 * as BLOB_SERVE_PROOF ledger entries — this is how hosts get paid for
 * bandwidth delivery in v2.11.1+.
 *
 * Not persistent across process restarts. By design: if a host crashes
 * before flushing, the uncommitted bytes are lost to the host (honest-
 * hosts-eat-the-loss rule). This motivates frequent flushes.
 *
 * Usage:
 *   const bw = require('./blobBandwidth');
 *   bw.accumulate(cid, bytesServed);      // called on every GET
 *   await bw.flushServeProofs(host);      // called on a schedule
 *   bw.getPending(cid);                   // introspection
 *
 * Chain invariants (enforced downstream in stateStore dispatcher):
 *   - Only CIDs with a prior BLOB_STORE_COMMIT produce valid proofs
 *   - Only committed hosts can submit valid proofs
 *   - Fraudulent inflation caught by v2.11.2 verifier spot-checks
 */

var pendingServed = new Map(); // cid -> { bytes, requests }

/**
 * Accumulate bytes served for a given CID. Idempotent at the byte
 * level — every call adds to the running total.
 */
function accumulate(cid, bytes) {
  if (typeof cid !== "string" || cid.length === 0) return;
  var n = Number(bytes);
  if (!isFinite(n) || n <= 0) return;
  var cur = pendingServed.get(cid) || { bytes: 0, requests: 0 };
  cur.bytes += n;
  cur.requests += 1;
  pendingServed.set(cid, cur);
}

/**
 * Get pending counts for a specific CID, or all pending if no CID given.
 */
function getPending(cid) {
  if (cid) {
    return pendingServed.get(cid) || { bytes: 0, requests: 0 };
  }
  var result = {};
  for (var entry of pendingServed) {
    result[entry[0]] = entry[1];
  }
  return result;
}

/**
 * Count of distinct CIDs with pending non-zero bytes.
 */
function pendingCidCount() {
  var count = 0;
  for (var entry of pendingServed) {
    if (entry[1].bytes > 0) count++;
  }
  return count;
}

/**
 * Clear pending counts for a specific CID without flushing. Used after
 * a successful chain submission to avoid double-counting.
 */
function clearPending(cid) {
  return pendingServed.delete(cid);
}

/**
 * Flush all pending serve counts to chain via BLOB_SERVE_PROOF entries.
 *
 * Requires an injected ledger + stateStore to record + query on chain.
 * This module can't require them directly because tests need to inject
 * mocks, and circular dependencies would form if we imported them here.
 *
 * Only flushes CIDs where:
 *   1. There's a commit on chain (stateStore.getBlobCommit)
 *   2. The flushing host is in the committed hosts list
 *   3. Pending bytes > 0
 *
 * Returns { flushed, skipped, details } where details is an array of
 * per-CID outcomes for debugging.
 */
async function flushServeProofs(hostUsername, options) {
  options = options || {};
  var ledger = options.ledger || require("./ledger");
  var stateStore = options.stateStore || require("../chain/stateStore");

  if (!hostUsername) {
    return { flushed: 0, skipped: 0, details: [] };
  }

  var flushed = 0;
  var skipped = 0;
  var details = [];
  var epoch = await ledger.getCurrentEpoch();
  var cids = Array.from(pendingServed.keys());

  for (var i = 0; i < cids.length; i++) {
    var cid = cids[i];
    var counts = pendingServed.get(cid);
    if (!counts || counts.bytes <= 0) {
      pendingServed.delete(cid);
      continue;
    }

    var commit = stateStore.getBlobCommit(cid);
    if (!commit) {
      skipped++;
      details.push({ cid: cid, status: "no_commit", bytes: counts.bytes });
      continue;
    }
    if (commit.hosts.indexOf(hostUsername) === -1) {
      skipped++;
      details.push({ cid: cid, status: "not_committed_host", bytes: counts.bytes });
      continue;
    }

    try {
      await ledger.recordBlobServeProof(
        hostUsername,
        cid,
        counts.bytes,
        counts.requests,
        null, // access_log_merkle_root wired in v2.11.2
        epoch
      );
      flushed++;
      details.push({
        cid: cid,
        status: "flushed",
        bytes: counts.bytes,
        requests: counts.requests,
      });
      pendingServed.delete(cid);
    } catch (err) {
      skipped++;
      details.push({ cid: cid, status: "error", error: err.message });
    }
  }

  return { flushed: flushed, skipped: skipped, details: details };
}

/**
 * Reset all pending state. For tests only.
 */
function resetForTests() {
  pendingServed.clear();
}

module.exports = {
  accumulate: accumulate,
  getPending: getPending,
  pendingCidCount: pendingCidCount,
  clearPending: clearPending,
  flushServeProofs: flushServeProofs,
  resetForTests: resetForTests,
};

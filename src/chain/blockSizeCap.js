"use strict";

/**
 * HONE Block Size Cap — v3.0
 * Shin Devlin
 *
 * Dynamic block cap: starts at 1 MB, adapts based on how full recent
 * blocks have been. Grows up to 32 MB when congested, shrinks back to
 * 1 MB when quiet.
 *
 * Adjustment rules (applied per epoch):
 *   Block was >50% full  → next cap = min(current × 1.25, 32 MB)
 *   Block was <25% full  → next cap = max(current × 0.75,  1 MB)
 *   Block was 25–50% full → cap unchanged
 *
 * At 32 MB × 2880 blocks/day × 365 days = ~33 TB/year max chain growth.
 * Default 1 MB × 2880 blocks/day = ~1 GB/day, trivially storable.
 *
 * Sibling module to blockStore.js — exports the cap + helpers without
 * touching the underlying disk-write logic. Callers (the miner, the
 * P2P block validator, the gateway) import this to estimate, trim, or
 * reject payloads BEFORE calling blockStore.writeBlock. Defense in
 * depth at the disk layer is recommended but not enforced here so the
 * v2.11 file format stays unchanged.
 */

// ─── Dynamic cap constants ────────────────────────────────────────

var DEFAULT_BLOCK_CAP = 1 * 1024 * 1024;  // 1 MB starting point
var MAX_BLOCK_CAP = 32 * 1024 * 1024;     // 32 MB ceiling
var MIN_BLOCK_CAP = 1 * 1024 * 1024;      // 1 MB floor

/**
 * Compute the cap for the NEXT block given the current cap and the
 * actual size of the block just produced.
 *
 *   >50% full → grow 25%  (capped at MAX_BLOCK_CAP)
 *   <25% full → shrink 25% (floored at MIN_BLOCK_CAP)
 *   25–50%   → unchanged
 */
function computeNextBlockCap(currentCap, currentBlockSize) {
  if (currentBlockSize > currentCap * 0.5) {
    return Math.min(Math.floor(currentCap * 1.25), MAX_BLOCK_CAP);
  }
  if (currentBlockSize < currentCap * 0.25) {
    return Math.max(Math.floor(currentCap * 0.75), MIN_BLOCK_CAP);
  }
  return currentCap;
}

// Legacy single-value export: the "current" cap. Callers that just want
// a number to check against can use this. The epoch manager updates
// currentBlockCap in stateStore; the miner reads it from there.
// For backward compat, this module also exports MAX_BLOCK_PAYLOAD_BYTES
// (initialized to DEFAULT_BLOCK_CAP) so old call sites don't break.
var MAX_BLOCK_PAYLOAD_BYTES = parseInt(
  process.env.HONE_MAX_BLOCK_PAYLOAD_BYTES || String(DEFAULT_BLOCK_CAP),
  10
);

/**
 * Estimate the JSON payload size of a candidate block payload.
 * Used by the miner to decide when to stop adding ledger entries.
 *
 * Returns the byte length of JSON.stringify(payload). Cheap.
 */
function estimateBlockPayloadSize(payload) {
  if (!payload) return 0;
  return Buffer.byteLength(JSON.stringify(payload), "utf8");
}

/**
 * How many bytes of headroom remain before a payload would exceed
 * the cap. Negative means it's already over.
 */
function getBlockSpaceRemaining(payload) {
  return MAX_BLOCK_PAYLOAD_BYTES - estimateBlockPayloadSize(payload);
}

/**
 * Check if a payload fits under the cap. Returns true if it does.
 */
function fitsUnderCap(payload) {
  return estimateBlockPayloadSize(payload) <= MAX_BLOCK_PAYLOAD_BYTES;
}

/**
 * Trim a list of ledger entries so the resulting payload fits within
 * the cap. Greedy and order-preserving: keeps entries from the front,
 * drops from the back when they would push over the cap. Continues
 * checking later entries (a smaller one may fit even after a larger
 * one was dropped) so the algorithm is best-effort packing.
 *
 * v2.12-beta will replace this with a fee-market-aware sorter that
 * prefers higher-fee entries when congested. For now the trim is
 * first-come-first-served (entries are processed in the order they
 * arrived in pendingEntries).
 *
 * Implementation: O(n) — pre-serialize each entry once, maintain a
 * running byte total instead of re-serializing the whole kept list
 * on every iteration. Earlier impl was O(n²) and timed out at 20k
 * entries.
 *
 * Returns { kept: [...], dropped: [...], final_bytes }
 */
function trimEntriesToCap(entries, basePayload) {
  basePayload = basePayload || {};
  // Compute base overhead: payload with empty ledger_entries
  var baseClone = Object.assign({}, basePayload, { ledger_entries: [] });
  var baseBytes = Buffer.byteLength(JSON.stringify(baseClone), "utf8");
  // The empty array `[]` contributes 2 bytes; each subsequent entry
  // adds its own JSON length plus a comma if not the first.
  // We compute: total = baseBytes - 2 + (new array bytes)
  // Where new array bytes = 2 (brackets) + sum(entry_lens) + (n-1) commas

  var kept = [];
  var dropped = [];
  var keptEntryBytes = 0; // sum of JSON lengths of kept entries (no commas)

  for (var i = 0; i < entries.length; i++) {
    var entryJson = JSON.stringify(entries[i]);
    var entryBytes = Buffer.byteLength(entryJson, "utf8");

    // If we add this entry, the new array would be:
    //   2 (brackets) + (keptEntryBytes + entryBytes) + (kept.length) commas
    // Total payload = baseBytes - 2 (the empty []) + new array bytes
    var newCommas = kept.length; // commas added between entries
    var newArrayBytes = 2 + keptEntryBytes + entryBytes + newCommas;
    var projectedTotal = baseBytes - 2 + newArrayBytes;

    if (projectedTotal <= MAX_BLOCK_PAYLOAD_BYTES) {
      kept.push(entries[i]);
      keptEntryBytes += entryBytes;
    } else {
      dropped.push(entries[i]);
    }
  }

  // Compute the actual final size (cheap, runs once)
  var finalPayload = Object.assign({}, basePayload, { ledger_entries: kept });
  return {
    kept: kept,
    dropped: dropped,
    final_bytes: Buffer.byteLength(JSON.stringify(finalPayload), "utf8"),
  };
}

/**
 * Validate a payload against the cap and throw on violation. Use this
 * before calling blockStore.writeBlock if you want to fail fast with a
 * clear error rather than silently trying to write an oversized file.
 */
function assertFitsUnderCap(payload) {
  var size = estimateBlockPayloadSize(payload);
  if (size > MAX_BLOCK_PAYLOAD_BYTES) {
    throw new Error(
      "block payload exceeds MAX_BLOCK_PAYLOAD_BYTES (" +
        size +
        " > " +
        MAX_BLOCK_PAYLOAD_BYTES +
        ")"
    );
  }
}

module.exports = {
  MAX_BLOCK_PAYLOAD_BYTES: MAX_BLOCK_PAYLOAD_BYTES,
  DEFAULT_BLOCK_CAP: DEFAULT_BLOCK_CAP,
  MAX_BLOCK_CAP: MAX_BLOCK_CAP,
  MIN_BLOCK_CAP: MIN_BLOCK_CAP,
  computeNextBlockCap: computeNextBlockCap,
  estimateBlockPayloadSize: estimateBlockPayloadSize,
  getBlockSpaceRemaining: getBlockSpaceRemaining,
  fitsUnderCap: fitsUnderCap,
  trimEntriesToCap: trimEntriesToCap,
  assertFitsUnderCap: assertFitsUnderCap,
};

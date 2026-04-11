"use strict";

/**
 * BTCPC Block Size Cap — v2.12-alpha
 * Shin Devlin
 *
 * Hard limit on the JSON payload size of any single block. 1 MB matches
 * Bitcoin's classic block-size policy and keeps replay times bounded as
 * the chain grows. Configurable via env var for downstream chains that
 * want different economics.
 *
 * At 1 MB per block × 288 blocks/day × 365 days = ~105 GB/year of chain
 * growth, which is trivially storable on a $50 HDD. Even 10x that
 * wouldn't strain a single full node.
 *
 * Sibling module to blockStore.js — exports the cap + helpers without
 * touching the underlying disk-write logic. Callers (the miner, the
 * P2P block validator, the gateway) import this to estimate, trim, or
 * reject payloads BEFORE calling blockStore.writeBlock. Defense in
 * depth at the disk layer is recommended but not enforced here so the
 * v2.11 file format stays unchanged.
 */

var MAX_BLOCK_PAYLOAD_BYTES = parseInt(
  process.env.BTCPC_MAX_BLOCK_PAYLOAD_BYTES || String(1 * 1024 * 1024),
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
  estimateBlockPayloadSize: estimateBlockPayloadSize,
  getBlockSpaceRemaining: getBlockSpaceRemaining,
  fitsUnderCap: fitsUnderCap,
  trimEntriesToCap: trimEntriesToCap,
  assertFitsUnderCap: assertFitsUnderCap,
};

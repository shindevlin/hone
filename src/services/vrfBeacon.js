"use strict";

/**
 * BTCPC VRF Beacon — v2.12-gamma
 * Shin Devlin
 *
 * Provably fair randomness for games, lotteries, NFT mints, and any
 * other on-chain primitive that needs unmanipulable random values.
 *
 * Design: a commit-reveal scheme that aggregates contributions from
 * multiple clock nodes per epoch and XORs them into a single beacon.
 * As long as ONE participating clock node is honest and unpredictable,
 * the beacon is unpredictable to everyone else — even the other clock
 * nodes can't bias it because they commit before seeing other reveals.
 *
 * Why not use sha256(block_hash) directly?
 * Block producers can grind block hashes by including or excluding
 * specific transactions. A miner with a stake in the random outcome
 * can re-mine until they get a favorable hash. VRF eliminates this
 * by separating the randomness commit from the block production.
 *
 * Why not use a chain-derived seed (block_hash + epoch)?
 * Same problem. Anything derivable from on-chain state at block-creation
 * time is grindable by the block creator.
 *
 * Why a commit-reveal scheme instead of ECVRF?
 * ECVRF is technically superior (single-shot proofs, no commit phase)
 * but requires a curve library (noble or similar) and per-node key
 * setup. Commit-reveal works with primitives we already have (sha256
 * + node signing) and gets us 95% of the security with 5% of the
 * implementation complexity. ECVRF can be a v2.12.x upgrade later
 * if needed.
 *
 * Trust model:
 *   - Each participating clock node generates a 32-byte secret per epoch
 *   - At commit phase: clock posts sha256(secret) on chain
 *   - At reveal phase: clock posts the secret itself
 *   - Beacon = XOR(all_revealed_secrets) for clocks whose reveal matches
 *     their commit
 *
 * As long as ONE clock node is honest and keeps their secret private
 * until reveal, no other party can predict or bias the beacon.
 */

var crypto = require("crypto");

/**
 * Generate a fresh 32-byte random secret. Each clock node calls this
 * once per epoch to get their VRF contribution.
 */
function generateSecret() {
  return crypto.randomBytes(32).toString("hex");
}

/**
 * Compute the commit hash for a secret. This is what the clock node
 * posts in the commit phase.
 */
function commitHash(secret) {
  if (typeof secret !== "string") {
    throw new Error("secret must be a hex string");
  }
  return crypto.createHash("sha256").update(secret, "hex").digest("hex");
}

/**
 * Verify that a revealed secret matches a previously-published commit.
 */
function verifyReveal(secret, commit) {
  if (!secret || !commit) return false;
  try {
    return commitHash(secret) === commit;
  } catch (_) {
    return false;
  }
}

/**
 * Compute the epoch beacon by XORing all valid (commit, reveal) pairs.
 *
 * Inputs: array of { node_id, commit, reveal } objects, where commit
 * is the sha256(secret) and reveal is the secret itself. Pairs whose
 * reveal does NOT match their commit are silently dropped — those
 * nodes are non-participants for this epoch.
 *
 * Returns the 32-byte XOR of all valid reveals as a hex string,
 * plus the list of contributors.
 *
 * If zero valid contributions, returns null (no beacon for this epoch).
 */
function computeBeacon(contributions) {
  if (!Array.isArray(contributions) || contributions.length === 0) {
    return { beacon: null, contributors: [], rejected: [] };
  }

  var xorBuffer = Buffer.alloc(32);
  var contributors = [];
  var rejected = [];

  for (var i = 0; i < contributions.length; i++) {
    var c = contributions[i];
    if (!c || !c.node_id || !c.commit || !c.reveal) {
      rejected.push({ node_id: c && c.node_id, reason: "missing fields" });
      continue;
    }
    if (!verifyReveal(c.reveal, c.commit)) {
      rejected.push({ node_id: c.node_id, reason: "reveal does not match commit" });
      continue;
    }
    var revealBuf = Buffer.from(c.reveal, "hex");
    if (revealBuf.length !== 32) {
      rejected.push({ node_id: c.node_id, reason: "reveal is not 32 bytes" });
      continue;
    }
    // XOR into running buffer
    for (var b = 0; b < 32; b++) {
      xorBuffer[b] = xorBuffer[b] ^ revealBuf[b];
    }
    contributors.push(c.node_id);
  }

  if (contributors.length === 0) {
    return { beacon: null, contributors: [], rejected: rejected };
  }

  return {
    beacon: xorBuffer.toString("hex"),
    contributors: contributors,
    rejected: rejected,
  };
}

/**
 * Derive a uniformly-distributed integer in [0, max) from a beacon.
 * Used by consumers (lotteries, games, NFT mints) to extract specific
 * values from the 32-byte beacon.
 *
 * The derivation includes a per-call salt so multiple consumers in the
 * same epoch get different values from the same beacon without
 * collisions.
 */
function deriveInt(beacon, max, salt) {
  if (!beacon || max <= 0) return 0;
  var input = beacon + ":" + (salt || "");
  var hash = crypto.createHash("sha256").update(input).digest();
  // Read first 8 bytes as a big-endian uint64, mod max
  var n = 0n;
  for (var i = 0; i < 8; i++) {
    n = (n << 8n) | BigInt(hash[i]);
  }
  return Number(n % BigInt(max));
}

/**
 * Derive a deterministic random index for selecting from a list.
 * Convenience wrapper around deriveInt.
 */
function deriveIndex(beacon, listLength, salt) {
  return deriveInt(beacon, listLength, salt);
}

/**
 * Derive a 0.0-1.0 float from a beacon. Useful for percentage rolls,
 * loot drop chances, etc.
 */
function deriveFloat(beacon, salt) {
  if (!beacon) return 0;
  var input = beacon + ":float:" + (salt || "");
  var hash = crypto.createHash("sha256").update(input).digest();
  // Use first 6 bytes for 48-bit precision (more than enough for floats)
  var n = 0;
  for (var i = 0; i < 6; i++) {
    n = n * 256 + hash[i];
  }
  return n / Math.pow(2, 48);
}

/**
 * Shuffle an array deterministically using a beacon. Used for things
 * like fair card decks, random ordering, lottery winner selection.
 *
 * Returns a NEW array; doesn't mutate the input.
 */
function shuffleArray(beacon, array, salt) {
  if (!Array.isArray(array)) return [];
  var result = array.slice();
  // Fisher-Yates with beacon-derived random indices
  for (var i = result.length - 1; i > 0; i--) {
    var j = deriveInt(beacon, i + 1, (salt || "shuffle") + ":" + i);
    var tmp = result[i];
    result[i] = result[j];
    result[j] = tmp;
  }
  return result;
}

module.exports = {
  generateSecret: generateSecret,
  commitHash: commitHash,
  verifyReveal: verifyReveal,
  computeBeacon: computeBeacon,
  deriveInt: deriveInt,
  deriveIndex: deriveIndex,
  deriveFloat: deriveFloat,
  shuffleArray: shuffleArray,
};

"use strict";

/**
 * BTCPC Block Serialization
 * Shin Devlin
 *
 * Formal block header structure per Whitepaper 2.2.
 * Provides binary serialization, SHA-256 hashing, Merkle tree computation,
 * and block validation for the BTCPC sovereign chain.
 */

var crypto = require("crypto");

// Header field sizes (bytes)
var VERSION_SIZE = 4;
var HASH_SIZE = 32;
var TIMESTAMP_SIZE = 8;
var EPOCH_SIZE = 4;
var DIFFICULTY_SIZE = 4;
var MINER_ID_SIZE = 32;

var HEADER_SIZE = VERSION_SIZE + HASH_SIZE + HASH_SIZE + HASH_SIZE +
  TIMESTAMP_SIZE + EPOCH_SIZE + DIFFICULTY_SIZE + MINER_ID_SIZE;
// 4 + 32 + 32 + 32 + 8 + 4 + 4 + 32 = 148 bytes

/**
 * Block — Represents a BTCPC block with formal binary serialization.
 *
 * Header fields (per Whitepaper 2.2):
 *   version                    — uint32
 *   previous_block_hash        — 32-byte hash
 *   merkle_root_transactions   — 32-byte hash
 *   merkle_root_compute_proofs — 32-byte hash
 *   timestamp                  — uint64 (ms since epoch)
 *   epoch_number               — uint32
 *   difficulty                 — uint32
 *   miner_id                   — 32-byte identifier
 */
function Block(opts) {
  opts = opts || {};
  this.version = opts.version || 1;
  this.previous_block_hash = opts.previous_block_hash || "0".repeat(64);
  this.merkle_root_transactions = opts.merkle_root_transactions || "0".repeat(64);
  this.merkle_root_compute_proofs = opts.merkle_root_compute_proofs || "0".repeat(64);
  this.timestamp = opts.timestamp || Date.now();
  this.epoch_number = opts.epoch_number || 0;
  this.difficulty = opts.difficulty || 1;
  this.miner_id = opts.miner_id || "0".repeat(64);

  // Non-header payload (not included in the header hash)
  this.transactions = opts.transactions || [];
  this.compute_proofs = opts.compute_proofs || [];
}

/**
 * serialize — Encode the block header into a fixed-size Buffer.
 * Returns a Buffer of HEADER_SIZE bytes.
 */
Block.prototype.serialize = function () {
  var buf = Buffer.alloc(HEADER_SIZE);
  var offset = 0;

  // version — 4 bytes, little-endian uint32
  buf.writeUInt32LE(this.version, offset);
  offset += VERSION_SIZE;

  // previous_block_hash — 32 bytes
  var prevHash = normalizeHash(this.previous_block_hash);
  prevHash.copy(buf, offset);
  offset += HASH_SIZE;

  // merkle_root_transactions — 32 bytes
  var txRoot = normalizeHash(this.merkle_root_transactions);
  txRoot.copy(buf, offset);
  offset += HASH_SIZE;

  // merkle_root_compute_proofs — 32 bytes
  var cpRoot = normalizeHash(this.merkle_root_compute_proofs);
  cpRoot.copy(buf, offset);
  offset += HASH_SIZE;

  // timestamp — 8 bytes, little-endian uint64
  // Use two 32-bit writes since Buffer has no writeUInt64
  var ts = typeof this.timestamp === "number" ? this.timestamp : Date.now();
  buf.writeUInt32LE(ts & 0xFFFFFFFF, offset);
  buf.writeUInt32LE(Math.floor(ts / 0x100000000) & 0xFFFFFFFF, offset + 4);
  offset += TIMESTAMP_SIZE;

  // epoch_number — 4 bytes, little-endian uint32
  buf.writeUInt32LE(this.epoch_number, offset);
  offset += EPOCH_SIZE;

  // difficulty — 4 bytes, little-endian uint32
  buf.writeUInt32LE(this.difficulty, offset);
  offset += DIFFICULTY_SIZE;

  // miner_id — 32 bytes (hash of the miner identifier string)
  var minerId = normalizeHash(this.miner_id);
  minerId.copy(buf, offset);

  return buf;
};

/**
 * deserialize — Decode a Buffer back into a Block instance.
 * Returns a new Block.
 */
Block.deserialize = function (buffer) {
  if (!Buffer.isBuffer(buffer)) {
    throw new Error("deserialize expects a Buffer");
  }
  if (buffer.length < HEADER_SIZE) {
    throw new Error("Buffer too small: expected " + HEADER_SIZE + " bytes, got " + buffer.length);
  }

  var offset = 0;

  var version = buffer.readUInt32LE(offset);
  offset += VERSION_SIZE;

  var previous_block_hash = buffer.slice(offset, offset + HASH_SIZE).toString("hex");
  offset += HASH_SIZE;

  var merkle_root_transactions = buffer.slice(offset, offset + HASH_SIZE).toString("hex");
  offset += HASH_SIZE;

  var merkle_root_compute_proofs = buffer.slice(offset, offset + HASH_SIZE).toString("hex");
  offset += HASH_SIZE;

  var tsLow = buffer.readUInt32LE(offset);
  var tsHigh = buffer.readUInt32LE(offset + 4);
  var timestamp = tsHigh * 0x100000000 + tsLow;
  offset += TIMESTAMP_SIZE;

  var epoch_number = buffer.readUInt32LE(offset);
  offset += EPOCH_SIZE;

  var difficulty = buffer.readUInt32LE(offset);
  offset += DIFFICULTY_SIZE;

  var miner_id = buffer.slice(offset, offset + MINER_ID_SIZE).toString("hex");

  return new Block({
    version: version,
    previous_block_hash: previous_block_hash,
    merkle_root_transactions: merkle_root_transactions,
    merkle_root_compute_proofs: merkle_root_compute_proofs,
    timestamp: timestamp,
    epoch_number: epoch_number,
    difficulty: difficulty,
    miner_id: miner_id
  });
};

/**
 * computeHash — SHA-256 of the serialized header.
 * Returns the hash as a 64-character hex string.
 */
Block.prototype.computeHash = function () {
  var header = this.serialize();
  return crypto.createHash("sha256").update(header).digest("hex");
};

/**
 * computeMerkleRoot — Build a Merkle tree from an array of hex hash strings.
 * Returns the root hash as a 64-character hex string.
 * If items is empty, returns the zero hash.
 */
Block.computeMerkleRoot = function (items) {
  if (!items || items.length === 0) {
    return "0".repeat(64);
  }

  // Start with the leaf hashes
  var level = items.map(function (item) {
    // If the item is already a 64-char hex hash, use it directly.
    // Otherwise hash the string representation.
    if (typeof item === "string" && /^[0-9a-f]{64}$/i.test(item)) {
      return item;
    }
    return crypto.createHash("sha256").update(String(item)).digest("hex");
  });

  // Build the tree bottom-up
  while (level.length > 1) {
    var nextLevel = [];
    for (var i = 0; i < level.length; i += 2) {
      var left = level[i];
      // If odd number of elements, duplicate the last one
      var right = (i + 1 < level.length) ? level[i + 1] : level[i];
      var combined = crypto.createHash("sha256")
        .update(left + right)
        .digest("hex");
      nextLevel.push(combined);
    }
    level = nextLevel;
  }

  return level[0];
};

/**
 * validateBlock — Verify a block against the previous block.
 * Checks: previous hash linkage, timestamp ordering, difficulty target.
 * Returns true if valid.
 */
Block.prototype.validateBlock = function (previousBlock) {
  // Previous hash must match
  if (previousBlock) {
    var expectedPrevHash = previousBlock.computeHash();
    if (this.previous_block_hash !== expectedPrevHash) {
      return false;
    }
  } else {
    // Genesis block — previous hash must be all zeros
    if (this.previous_block_hash !== "0".repeat(64)) {
      return false;
    }
  }

  // Timestamp must be after previous block
  if (previousBlock && this.timestamp <= previousBlock.timestamp) {
    return false;
  }

  // Timestamp must not be too far in the future (2 hours tolerance)
  var maxFutureMs = 2 * 60 * 60 * 1000;
  if (this.timestamp > Date.now() + maxFutureMs) {
    return false;
  }

  // Difficulty must be positive
  if (this.difficulty < 1) {
    return false;
  }

  // Epoch number must be sequential
  if (previousBlock) {
    if (this.epoch_number !== previousBlock.epoch_number + 1) {
      return false;
    }
  }

  return true;
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Normalize a hash string or identifier to a 32-byte Buffer.
 * If the input is a 64-char hex string, decode it directly.
 * Otherwise, SHA-256 hash the input string and return the digest.
 */
function normalizeHash(input) {
  if (typeof input === "string" && /^[0-9a-f]{64}$/i.test(input)) {
    return Buffer.from(input, "hex");
  }
  // Hash the string to get a deterministic 32-byte value
  return crypto.createHash("sha256").update(String(input || "")).digest();
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

Block.HEADER_SIZE = HEADER_SIZE;

module.exports = Block;

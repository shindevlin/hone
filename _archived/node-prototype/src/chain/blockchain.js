"use strict";

/**
 * HONE Blockchain
 * Shin Devlin
 *
 * In-memory blockchain state backed by the Block serialization layer.
 * Provides block storage, retrieval, and full-chain validation.
 */

var Block = require("./block");

// Storage — keyed by hash and by epoch number
var blocksByHash = new Map();
var blocksByEpoch = new Map();
var latestBlock = null;

/**
 * addBlock — Validate a block against the current tip and store it.
 * Returns true if the block was accepted, false if rejected.
 */
function addBlock(block) {
  if (!(block instanceof Block)) {
    // Allow plain objects — wrap them
    block = new Block(block);
  }

  // Validate against the previous block
  if (!block.validateBlock(latestBlock)) {
    return false;
  }

  var hash = block.computeHash();

  // Reject duplicates
  if (blocksByHash.has(hash)) {
    return false;
  }

  blocksByHash.set(hash, block);
  blocksByEpoch.set(block.epoch_number, block);
  latestBlock = block;

  return true;
}

/**
 * getBlock — Retrieve a block by its SHA-256 hash.
 * Returns the Block or null.
 */
function getBlock(hash) {
  return blocksByHash.get(hash) || null;
}

/**
 * getBlockByEpoch — Retrieve a block by its epoch number.
 * Returns the Block or null.
 */
function getBlockByEpoch(epoch) {
  return blocksByEpoch.get(epoch) || null;
}

/**
 * getLatestBlock — Return the current chain tip.
 * Returns the Block or null if the chain is empty.
 */
function getLatestBlock() {
  return latestBlock;
}

/**
 * getChainHeight — Return the current chain height (latest epoch number).
 * Returns -1 if the chain is empty.
 */
function getChainHeight() {
  if (!latestBlock) return -1;
  return latestBlock.epoch_number;
}

/**
 * validateChain — Walk the entire chain from genesis to tip and verify integrity.
 * Returns { valid: bool, height: number, error: string|null }
 */
function validateChain() {
  if (blocksByEpoch.size === 0) {
    return { valid: true, height: -1, error: null };
  }

  var height = latestBlock ? latestBlock.epoch_number : -1;

  // Walk from epoch 0 to tip
  var prev = null;
  for (var i = 0; i <= height; i++) {
    var block = blocksByEpoch.get(i);
    if (!block) {
      return {
        valid: false,
        height: height,
        error: "Missing block at epoch " + i
      };
    }

    if (!block.validateBlock(prev)) {
      return {
        valid: false,
        height: height,
        error: "Invalid block at epoch " + i
      };
    }

    // Verify stored hash matches computed hash
    var computedHash = block.computeHash();
    if (!blocksByHash.has(computedHash)) {
      return {
        valid: false,
        height: height,
        error: "Hash mismatch at epoch " + i
      };
    }

    prev = block;
  }

  return { valid: true, height: height, error: null };
}

/**
 * getBlockCount — Return the total number of stored blocks.
 */
function getBlockCount() {
  return blocksByHash.size;
}

/**
 * reset — Clear all stored blocks (for testing).
 */
function reset() {
  blocksByHash.clear();
  blocksByEpoch.clear();
  latestBlock = null;
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  addBlock: addBlock,
  getBlock: getBlock,
  getBlockByEpoch: getBlockByEpoch,
  getLatestBlock: getLatestBlock,
  getChainHeight: getChainHeight,
  validateChain: validateChain,
  getBlockCount: getBlockCount,
  reset: reset
};

"use strict";

/**
 * BTCPC Chain Sync
 * Shin Devlin
 *
 * Synchronizes chain state with peers. Handles block validation,
 * chain height tracking, and serving block ranges for peer sync.
 */

const crypto = require("crypto");

// In-memory chain state cache
// In production this is backed by MongoDB (Epoch model), but for the P2P layer
// we maintain a fast in-memory index keyed by epoch number.
const blockCache = new Map();
let chainHeight = -1;

// ---------------------------------------------------------------------------
// Chain height
// ---------------------------------------------------------------------------

/**
 * Get the current highest finalized epoch number.
 */
function getChainHeight() {
  return chainHeight;
}

/**
 * Set the chain height (called when epochs are loaded from DB or synced).
 */
function setChainHeight(height) {
  if (height > chainHeight) {
    chainHeight = height;
  }
}

// ---------------------------------------------------------------------------
// Block cache
// ---------------------------------------------------------------------------

/**
 * Store a block in the in-memory cache.
 */
function cacheBlock(block) {
  if (!block || block.epoch_number === undefined) return;
  blockCache.set(block.epoch_number, block);
  if (block.epoch_number > chainHeight) {
    chainHeight = block.epoch_number;
  }
}

/**
 * Get a block from the cache by epoch number.
 */
function getCachedBlock(epochNumber) {
  return blockCache.get(epochNumber) || null;
}

// ---------------------------------------------------------------------------
// Block validation
// ---------------------------------------------------------------------------

/**
 * Validate a block's integrity.
 * Checks: epoch_number exists, state_hash present, reward is non-negative.
 * Returns true if valid, false otherwise.
 */
function validateBlock(block) {
  if (!block) return false;

  // Must have an epoch number
  if (block.epoch_number === undefined || block.epoch_number === null) {
    return false;
  }

  if (typeof block.epoch_number !== "number" || block.epoch_number < 0) {
    return false;
  }

  // Must have a consensus/state hash (64-char hex or null for pending)
  if (block.consensus_hash) {
    if (typeof block.consensus_hash !== "string") return false;
    if (!/^[0-9a-f]{64}$/i.test(block.consensus_hash)) return false;
  }

  // Block reward must be non-negative
  if (block.block_reward !== undefined) {
    if (typeof block.block_reward !== "number" || block.block_reward < 0) {
      return false;
    }
  }

  // Status must be valid
  if (block.status) {
    if (!["active", "committed", "finalized"].includes(block.status)) {
      return false;
    }
  }

  // If block passes validation, cache it
  cacheBlock(block);

  return true;
}

// ---------------------------------------------------------------------------
// Block range serving
// ---------------------------------------------------------------------------

/**
 * Return blocks in the given epoch range (inclusive) for serving to peers.
 * Pulls from in-memory cache.
 */
function getBlockRange(fromEpoch, toEpoch) {
  const blocks = [];
  const start = Math.max(0, fromEpoch);
  const end = Math.min(toEpoch, chainHeight);

  for (let i = start; i <= end; i++) {
    const block = blockCache.get(i);
    if (block) {
      blocks.push(block);
    }
  }

  return blocks;
}

// ---------------------------------------------------------------------------
// Sync with peer
// ---------------------------------------------------------------------------

/**
 * Compare our chain height with a peer's and determine what blocks we need.
 * Returns { needSync: bool, localHeight, remoteHeight, missingFrom, missingTo }
 */
function comparePeer(peerChainHeight) {
  const local = chainHeight;
  const remote = peerChainHeight;

  if (remote <= local) {
    return {
      needSync: false,
      localHeight: local,
      remoteHeight: remote,
      missingFrom: null,
      missingTo: null
    };
  }

  return {
    needSync: true,
    localHeight: local,
    remoteHeight: remote,
    missingFrom: local + 1,
    missingTo: remote
  };
}

/**
 * Load finalized epochs from the database into the block cache.
 * Called on startup to initialize the chain state.
 */
async function loadFromDatabase() {
  try {
    const Epoch = require("../models/Epoch");
    const epochs = await Epoch.find({ status: "finalized" }).sort({ epoch_number: 1 });

    for (const epoch of epochs) {
      const block = {
        epoch_number: epoch.epoch_number,
        started_at: epoch.started_at,
        ended_at: epoch.ended_at,
        block_reward: epoch.block_reward,
        total_work: epoch.total_work,
        consensus_hash: epoch.consensus_hash,
        commitments: epoch.commitments,
        rewards_distributed: epoch.rewards_distributed,
        status: epoch.status
      };
      cacheBlock(block);
    }

    // Also check for active epochs to get the true latest
    const latestActive = await Epoch.findOne({ status: "active" }).sort({ epoch_number: -1 });
    if (latestActive && latestActive.epoch_number > chainHeight) {
      chainHeight = latestActive.epoch_number;
    }

    console.log("[BTCPC P2P] Chain loaded: " + blockCache.size + " blocks cached, height=" + chainHeight);
  } catch (err) {
    console.error("[BTCPC P2P] Failed to load chain from database:", err.message);
  }
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  getChainHeight,
  setChainHeight,
  validateBlock,
  getBlockRange,
  comparePeer,
  cacheBlock,
  getCachedBlock,
  loadFromDatabase
};

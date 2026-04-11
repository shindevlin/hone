"use strict";

/**
 * BTCPC Chain Sync
 * Shin Devlin
 *
 * Synchronizes chain state with peers. Handles block validation,
 * chain height tracking, and serving block ranges for peer sync.
 */

const crypto = require("crypto");
const blockStore = require("../chain/blockStore");
const Block = require("../chain/block");
const blockchain = require("../chain/blockchain");
const stateManager = require("../chain/stateManager");

// In-memory chain state cache
// Backed by block files on disk (source of truth) and MongoDB (cache).
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
 * Load chain from disk first, then fall back to MongoDB.
 * Block files on disk are the source of truth.
 */
async function loadFromDatabase() {
  try {
    blockStore.ensureBlockDir();

    // ── Try loading from block files on disk first ──
    const latestOnDisk = blockStore.getLatestBlockNumber();

    if (latestOnDisk >= 0) {
      console.log("[BTCPC P2P] Loading chain from disk (blocks 0-" + latestOnDisk + ")...");

      // Find latest finality block for fast SMT restore
      const latestFinality = blockStore.getLatestFinalityNumber();
      let replayFrom = 0;

      if (latestFinality >= 0) {
        const finData = blockStore.readFinality(latestFinality);
        if (finData && finData.snapshot) {
          stateManager.loadFromFinality(finData.snapshot);
          replayFrom = latestFinality + 1;
          console.log("[BTCPC P2P]   Loaded finality block at epoch " + latestFinality + " (" + (finData.snapshot.account_count || 0) + " accounts)");
        }
      }

      // Replay blocks from finality (or genesis) to rebuild state
      blockStore.iterateBlocks(replayFrom, latestOnDisk, function (block, payload, epoch) {
        // Apply ledger entries to SMT (skip if already loaded from finality at this epoch)
        if (epoch > latestFinality && payload.ledger_entries) {
          stateManager.applyLedgerEntries(payload.ledger_entries);
        }

        // Cache for P2P serving
        cacheBlock({
          epoch_number: block.epoch_number,
          block_reward: 0,
          total_work: 0,
          consensus_hash: block.state_root,
          status: "finalized",
          header_hex: block.serialize().toString("hex"),
          block_hash: block.computeHash()
        });

        // Add to formal blockchain
        blockchain.addBlock(block);
      });

      console.log("[BTCPC P2P] Chain loaded from disk: " + blockCache.size + " blocks, height=" + chainHeight + " | state root: " + stateManager.getStateRoot().slice(0, 16) + "...");
      return;
    }

    // Phase E: MongoDB Epoch fallback removed — block files are the source of truth.
    // If no block files exist on disk, the chain starts fresh from genesis.
    console.log("[BTCPC P2P] No block files on disk — chain starts fresh from genesis.");
  } catch (err) {
    console.error("[BTCPC P2P] Failed to load chain:", err.message);
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

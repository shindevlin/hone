"use strict";

/**
 * HONE Chain Replay
 * Shin Devlin
 *
 * Reconstructs in-memory chain state (stateStore + stateManager SMT +
 * nodeRegistry) from block files on disk. Called at startup.
 *
 * The blockchain is the canonical source of truth. This function takes
 * the latest finality snapshot (if any) as a fast-forward checkpoint,
 * then replays every block after that into stateStore.
 *
 * Follows the `nodeRegistry.loadFromBlocks()` pattern but for all state.
 */

var stateStore = require("./stateStore");
var stateManager = require("./stateManager");
var nodeRegistry = require("./nodeRegistry");
var blockStore = require("./blockStore");

/**
 * Replay all block files from disk into stateStore.
 *
 * @param {object} options
 * @param {boolean} [options.verbose=true] — log progress
 * @param {number} [options.from=0] — start epoch (ignored if finality snapshot available)
 * @returns {{ replayed, from, to, chainHeight, accounts, durationMs }}
 */
async function replayFromDisk(options) {
  options = options || {};
  var verbose = options.verbose !== false;
  var startTime = Date.now();

  // Reset all state before replay
  stateStore.resetAll();
  stateManager.reset();

  // Note: nodeRegistry doesn't have a public reset, but loadFromBlocks is idempotent
  // because it uses a Map and re-registers nodes.

  var latestOnDisk = blockStore.getLatestBlockNumber();
  if (latestOnDisk < 0) {
    if (verbose) console.log("[HONE Replay] No blocks on disk — fresh chain");
    return { replayed: 0, from: 0, to: -1, chainHeight: -1, accounts: 0, durationMs: 0 };
  }

  // Fast-forward via latest finality snapshot if available
  var latestFinality = blockStore.getLatestFinalityNumber();
  var replayFrom = 0;

  if (latestFinality >= 0) {
    var finality = blockStore.readFinality(latestFinality);
    if (finality && finality.snapshot) {
      try {
        stateManager.loadFromFinality(finality.snapshot);
        stateStore.hydrateFromFinality(finality.snapshot);
        stateStore.assertBalanceIntegrity("replay finality snapshot");
        replayFrom = latestFinality + 1;
        if (verbose) {
          console.log("[HONE Replay] Loaded finality snapshot at epoch " + latestFinality +
            " (" + (finality.snapshot.account_count || 0) + " accounts)");
        }
      } catch (e) {
        if (verbose) console.log("[HONE Replay] Failed to load finality " + latestFinality + ": " + e.message + " — replaying from genesis");
        stateStore.resetAll();
        stateManager.reset();
        replayFrom = 0;
      }
    }
  }

  // Replay blocks from replayFrom to latestOnDisk
  var blocksSeen = 0;
  var entriesSeen = 0;

  blockStore.iterateBlocks(replayFrom, latestOnDisk, function (block, payload, epochNumber) {
    blocksSeen++;

    if (!payload) return;

    // Apply ledger entries to stateStore (updates balances, accounts, tokens, etc.)
    if (Array.isArray(payload.ledger_entries)) {
      stateStore.applyEntries(payload.ledger_entries);
      entriesSeen += payload.ledger_entries.length;

      // Also apply to stateManager so the SMT tracks along with stateStore
      stateManager.applyLedgerEntries(payload.ledger_entries);

      // Nodes: forward NODE_REGISTER / STAKE / UNSTAKE to nodeRegistry
      for (var i = 0; i < payload.ledger_entries.length; i++) {
        var entry = payload.ledger_entries[i];
        if (entry.type === "NODE_REGISTER" ||
            entry.type === "STAKE" ||
            entry.type === "UNSTAKE" ||
            entry.type === "MINING_REWARD") {
          try { nodeRegistry.processLedgerEntry(entry); } catch (_) {}
        }
        // Index sensor readings so purchase rewards can find the owner later
        if (entry.type === "SENSOR_READING") {
          try {
            var sr = require("./sensorRewards");
            sr.indexReading(entry);
          } catch (_) {}
          // Note: rewardSettlement.recordSensorReading() is called from stateStore.applyEntries()
        }
      }
    }

    // Record mining proofs and compute proofs under their epoch
    if (Array.isArray(payload.mining_proofs)) {
      stateStore.setMiningProofs(epochNumber, payload.mining_proofs);
    }
    if (Array.isArray(payload.compute_proofs)) {
      stateStore.setComputeProofs(epochNumber, payload.compute_proofs);
    }

    // Record epoch metadata from the block header
    stateStore.setEpoch(epochNumber, {
      started_at: block.timestamp ? new Date(block.timestamp) : null,
      ended_at: block.timestamp ? new Date(block.timestamp) : null,
      consensus_hash: payload.consensus_hash || (block.computeHash ? block.computeHash() : null),
      state_root: payload.state_root || block.state_root || null,
      status: "finalized",
      rewards_distributed: payload.rewards || [],
      block_reward: payload.block_reward || 0,
      total_work: payload.total_work || 0,
      miner_id: block.miner_id || null,
    });
  });

  stateStore.setChainHeight(latestOnDisk);
  stateStore.assertBalanceIntegrity("replay complete");

  var duration = Date.now() - startTime;
  if (verbose) {
    console.log("[HONE Replay] Done: " + blocksSeen + " blocks (from " + replayFrom +
      " to " + latestOnDisk + "), " + entriesSeen + " ledger entries, " +
      stateStore.getAccountCount() + " accounts, " + duration + "ms");
  }

  return {
    replayed: blocksSeen,
    from: replayFrom,
    to: latestOnDisk,
    chainHeight: latestOnDisk,
    accounts: stateStore.getAccountCount(),
    entries: entriesSeen,
    durationMs: duration,
  };
}

module.exports = {
  replayFromDisk: replayFromDisk,
};

"use strict";

/**
 * HONE Node Registry — Lightweight, No MongoDB
 * Shin Devlin
 *
 * Reads registered nodes from block files and in-memory state.
 * Works without MongoDB — suitable for clock nodes.
 *
 * Node types:
 *   miner — full mining node (requires stake, GPU)
 *   clock — lightweight clock node
 *
 * Permission tiers:
 *   permissioned — approved by genesis operator, no stake required
 *   permissionless — anyone can join, must stake to get authority slots
 */

var blockStore = require("./blockStore");

// Minimum stake for permissionless nodes to participate in authority rotation
var PERMISSIONLESS_MIN_STAKE = 100; // HONE

// In-memory registry: username → { type, stake, p2pAddress, registeredEpoch, permissioned }
var nodes = new Map();

/**
 * Register a node. Called when processing NODE_REGISTER ledger entries.
 * @param {string} username
 * @param {string|string[]} nodeType — primary type string or array of types
 * @param {number} stake
 * @param {string} p2pAddress
 * @param {number} epoch
 * @param {boolean} permissioned
 */
function registerNode(username, nodeType, stake, p2pAddress, epoch, permissioned) {
  var types = Array.isArray(nodeType) ? nodeType : [nodeType || "clock"];
  nodes.set(username, {
    username: username,
    type: types[0] || "clock",
    node_types: types,
    stake: stake || 0,
    p2pAddress: p2pAddress || null,
    registeredEpoch: epoch || 0,
    permissioned: !!permissioned
  });
}

/**
 * Update a node's stake (called when STAKE/UNSTAKE entries are processed).
 */
function updateStake(username, newStake) {
  var node = nodes.get(username);
  if (node) {
    node.stake = newStake;
  }
}

/**
 * Get all registered nodes, sorted by stake (descending), then by username (alphabetical).
 * This ordering must be deterministic across all nodes.
 */
function getRegisteredNodes() {
  var list = Array.from(nodes.values());

  list.sort(function (a, b) {
    // Higher stake first
    if (b.stake !== a.stake) return b.stake - a.stake;
    // Alphabetical tiebreaker (deterministic)
    return a.username < b.username ? -1 : a.username > b.username ? 1 : 0;
  });

  return list;
}

/**
 * Check if a username is registered.
 */
function isRegistered(username) {
  return nodes.has(username);
}

/**
 * Get a specific node's info.
 */
function getNode(username) {
  return nodes.get(username) || null;
}

/**
 * Get the count of registered nodes.
 */
function getNodeCount() {
  return nodes.size;
}

/**
 * Load registry from block files on disk.
 * Scans all blocks for NODE_REGISTER and ACCOUNT_CREATE entries.
 */
function loadFromBlocks() {
  var latest = blockStore.getLatestBlockNumber();
  if (latest < 0) return;

  blockStore.iterateBlocks(0, latest, function (block, payload) {
    if (!payload || !payload.ledger_entries) return;

    for (var i = 0; i < payload.ledger_entries.length; i++) {
      var entry = payload.ledger_entries[i];
      processLedgerEntry(entry);
    }
  });
}

/**
 * Process a single ledger entry to update the registry.
 * Called during block replay and when new blocks arrive.
 */
function processLedgerEntry(entry) {
  if (!entry) return;

  switch (entry.type) {
    case "NODE_REGISTER": {
      var accountData = entry.account_data || {};
      // Support node_types array (multi-role) or legacy node_type string
      var nodeTypes = accountData.node_types || (accountData.node_type ? [accountData.node_type] : null)
        || (entry.memo ? [entry.memo] : null) || ["clock"];
      registerNode(
        entry.from || accountData.username,
        nodeTypes,
        0,
        accountData.p2p_address,
        entry.epoch,
        accountData.permissioned
      );
      break;
    }

    case "MINING_REWARD":
      // Any account that receives a mining reward is implicitly a miner node
      if (entry.to) {
        var existing = nodes.get(entry.to);
        if (!existing) {
          registerNode(entry.to, ["miner"], 0, null, entry.epoch);
        } else if (existing.node_types && existing.node_types.indexOf("miner") === -1) {
          // Add miner role to existing non-miner node
          existing.node_types = existing.node_types.concat(["miner"]);
        }
      }
      break;

    case "STAKE":
      // Track stake amounts
      if (entry.from) {
        var stakeNode = nodes.get(entry.from);
        if (stakeNode) {
          stakeNode.stake = (stakeNode.stake || 0) + (entry.amount || 0);
        }
      }
      break;

    case "UNSTAKE":
      if (entry.to) {
        var unstakeNode = nodes.get(entry.to);
        if (unstakeNode) {
          unstakeNode.stake = Math.max(0, (unstakeNode.stake || 0) - (entry.amount || 0));
        }
      }
      break;
  }
}

/**
 * Process an array of ledger entries (from a new block).
 */
function processLedgerEntries(entries) {
  if (!entries) return;
  for (var i = 0; i < entries.length; i++) {
    processLedgerEntry(entries[i]);
  }
}

/**
 * Reset the registry (for testing).
 */
function reset() {
  nodes.clear();
}

module.exports = {
  registerNode: registerNode,
  updateStake: updateStake,
  getRegisteredNodes: getRegisteredNodes,
  isRegistered: isRegistered,
  getNode: getNode,
  getNodeCount: getNodeCount,
  loadFromBlocks: loadFromBlocks,
  processLedgerEntry: processLedgerEntry,
  processLedgerEntries: processLedgerEntries,
  reset: reset,
  PERMISSIONLESS_MIN_STAKE: PERMISSIONLESS_MIN_STAKE
};

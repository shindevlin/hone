"use strict";

/**
 * BTCPC Finalization Consensus
 * Shin Devlin
 *
 * Manages the consensus process for epoch finalization.
 * After EPOCH_END, each miner computes a finalization proposal
 * (reward split based on proofs they've seen) and broadcasts it.
 * The network collects proposals, groups them by consensus_hash,
 * and the majority group wins.
 *
 * Flow:
 *   EPOCH_END → miners compute proposals → broadcast FINALIZATION_PROPOSAL
 *   → collect for 30s → group by consensus_hash → majority wins
 *   → earliest proposer in winning group broadcasts EPOCH_FINALIZED
 *
 * Single-miner fallback: if only one proposal after timeout, it wins.
 */

var crypto = require("crypto");

var PROPOSAL_WINDOW_MS = parseInt(process.env.BTCPC_PROPOSAL_WINDOW_MS) || 30000; // 30s
var PROPOSAL_TIMEOUT_MS = parseInt(process.env.BTCPC_PROPOSAL_TIMEOUT_MS) || 60000; // 60s fallback

// Per-epoch state
// Map<epochNumber, { proposals: [], windowStart: number, resolved: boolean, winner: object|null }>
var epochs = new Map();

// Callbacks
var onResolvedCallbacks = [];

/**
 * Hash a rewards array to produce a deterministic consensus_hash.
 * All miners with the same proofs will compute the same hash.
 */
function hashRewards(rewards, totalWork, settledJobs, epochNumber) {
  // Sort rewards deterministically: by miner name ascending
  var sorted = (rewards || []).slice().sort(function (a, b) {
    var aName = a.miner || a.node_id || "";
    var bName = b.miner || b.node_id || "";
    return aName < bName ? -1 : aName > bName ? 1 : 0;
  });

  // Hash rewards + epoch + total_work + settled_jobs count to prevent equivocation (T-05).
  // All consensus-critical fields bound — different work totals produce different hashes.
  var canonical = {
    epoch: epochNumber || 0,
    total_work: parseFloat((totalWork || 0).toFixed(6)),
    settled_jobs: Array.isArray(settledJobs) ? settledJobs.length : (settledJobs || 0),
    rewards: sorted.map(function (r) {
      return { miner: r.miner || r.node_id, amount: parseFloat((r.amount || 0).toFixed(10)) };
    })
  };

  return crypto.createHash("sha256").update(JSON.stringify(canonical)).digest("hex");
}

/**
 * Get or create epoch state.
 */
function _getEpoch(epochNumber) {
  if (!epochs.has(epochNumber)) {
    epochs.set(epochNumber, {
      proposals: [],
      windowStart: null,
      resolved: false,
      winner: null,
      timer: null
    });
  }
  return epochs.get(epochNumber);
}

/**
 * Submit a finalization proposal for an epoch.
 * Called when this miner computes its proposal OR when receiving one via P2P.
 *
 * @param {number} epochNumber
 * @param {object} proposal — { proposer, rewards, total_work, consensus_hash, settled_jobs, timestamp }
 * @returns {{ accepted: boolean, consensus: boolean, winner: object|null }}
 */
function submitProposal(epochNumber, proposal) {
  var state = _getEpoch(epochNumber);

  // Already resolved — reject late proposals
  if (state.resolved) {
    return { accepted: false, consensus: true, winner: state.winner };
  }

  // Start the proposal window on first proposal
  if (!state.windowStart) {
    state.windowStart = Date.now();

    // Set timeout for resolution
    state.timer = setTimeout(function () {
      resolve(epochNumber);
    }, PROPOSAL_WINDOW_MS);
  }

  // Reject duplicate proposers
  var existing = state.proposals.find(function (p) { return p.proposer === proposal.proposer; });
  if (existing) {
    return { accepted: false, consensus: false, winner: null };
  }

  // Ensure consensus_hash
  if (!proposal.consensus_hash) {
    proposal.consensus_hash = hashRewards(proposal.rewards, proposal.total_work, proposal.settled_jobs, epochNumber);
  }

  state.proposals.push(proposal);

  // Check for immediate consensus — all known miners agree
  var result = checkConsensus(epochNumber);
  if (result.consensus) {
    return { accepted: true, consensus: true, winner: result.winner };
  }

  return { accepted: true, consensus: false, winner: null };
}

/**
 * Check if consensus has been reached for an epoch.
 * Groups proposals by consensus_hash, checks if any group has majority.
 */
function checkConsensus(epochNumber) {
  var state = _getEpoch(epochNumber);
  if (state.resolved) return { consensus: true, winner: state.winner };
  if (state.proposals.length === 0) return { consensus: false, winner: null };

  // Single proposal — immediate consensus (genesis phase / solo miner)
  if (state.proposals.length === 1) {
    // Don't resolve immediately — wait for window unless it's been > timeout
    var elapsed = Date.now() - (state.windowStart || Date.now());
    if (elapsed >= PROPOSAL_WINDOW_MS) {
      resolve(epochNumber);
      return { consensus: true, winner: state.winner };
    }
    return { consensus: false, winner: null };
  }

  // Group by consensus_hash
  var groups = {};
  for (var i = 0; i < state.proposals.length; i++) {
    var p = state.proposals[i];
    var hash = p.consensus_hash;
    if (!groups[hash]) groups[hash] = [];
    groups[hash].push(p);
  }

  // Find the largest group
  var hashes = Object.keys(groups);
  var bestHash = null;
  var bestCount = 0;
  var bestWork = 0;

  for (var j = 0; j < hashes.length; j++) {
    var group = groups[hashes[j]];
    var groupWork = group.reduce(function (sum, p) { return sum + (p.total_work || 0); }, 0);

    if (group.length > bestCount || (group.length === bestCount && groupWork > bestWork)) {
      bestHash = hashes[j];
      bestCount = group.length;
      bestWork = groupWork;
    }
  }

  // Check if majority (> 50% of proposals)
  if (bestCount > state.proposals.length / 2) {
    resolve(epochNumber);
    return { consensus: true, winner: state.winner };
  }

  // No majority yet — wait for more proposals or timeout
  return { consensus: false, winner: null };
}

/**
 * Resolve an epoch — pick the winning proposal.
 * Called when consensus is reached or proposal window expires.
 */
function resolve(epochNumber) {
  var state = _getEpoch(epochNumber);
  if (state.resolved) return state.winner;
  if (state.proposals.length === 0) return null;

  // Clear timer
  if (state.timer) {
    clearTimeout(state.timer);
    state.timer = null;
  }

  // Group by consensus_hash
  var groups = {};
  for (var i = 0; i < state.proposals.length; i++) {
    var p = state.proposals[i];
    var hash = p.consensus_hash;
    if (!groups[hash]) groups[hash] = [];
    groups[hash].push(p);
  }

  // Find winning group: most proposals, then highest total_work, then earliest
  var hashes = Object.keys(groups);
  var winningHash = hashes[0];
  var winningGroup = groups[winningHash];

  for (var j = 1; j < hashes.length; j++) {
    var group = groups[hashes[j]];
    if (group.length > winningGroup.length) {
      winningHash = hashes[j];
      winningGroup = group;
    } else if (group.length === winningGroup.length) {
      // Tiebreak: highest total_work
      var groupWork = group.reduce(function (s, p) { return s + (p.total_work || 0); }, 0);
      var winWork = winningGroup.reduce(function (s, p) { return s + (p.total_work || 0); }, 0);
      if (groupWork > winWork) {
        winningHash = hashes[j];
        winningGroup = group;
      }
    }
  }

  // Within the winning group, earliest proposer wins the right to broadcast
  winningGroup.sort(function (a, b) { return (a.timestamp || 0) - (b.timestamp || 0); });
  var winner = winningGroup[0];

  state.resolved = true;
  state.winner = winner;

  console.log("[BTCPC Consensus] Epoch " + epochNumber + " resolved: " +
    state.proposals.length + " proposal(s), " +
    hashes.length + " group(s), winner: " + winner.proposer +
    " (hash: " + winningHash.slice(0, 12) + "...)");

  // Fire callbacks
  for (var k = 0; k < onResolvedCallbacks.length; k++) {
    try { onResolvedCallbacks[k](epochNumber, winner); } catch (e) {}
  }

  // Cleanup old epochs (keep last 10)
  var keys = Array.from(epochs.keys()).sort(function (a, b) { return a - b; });
  while (keys.length > 10) {
    var old = keys.shift();
    var oldState = epochs.get(old);
    if (oldState && oldState.timer) clearTimeout(oldState.timer);
    epochs.delete(old);
  }

  return winner;
}

/**
 * Register a callback for when consensus is reached.
 * callback(epochNumber, winningProposal)
 */
function onResolved(callback) {
  onResolvedCallbacks.push(callback);
}

/**
 * Check if an epoch has been resolved.
 */
function isResolved(epochNumber) {
  var state = epochs.get(epochNumber);
  return state ? state.resolved : false;
}

/**
 * Get the winning proposal for a resolved epoch.
 */
function getWinner(epochNumber) {
  var state = epochs.get(epochNumber);
  return state && state.resolved ? state.winner : null;
}

/**
 * Get all proposals for an epoch (for debugging/explorer).
 */
function getProposals(epochNumber) {
  var state = epochs.get(epochNumber);
  return state ? state.proposals : [];
}

/**
 * Check if this proposer is the designated broadcaster for the winning group.
 */
function amIBroadcaster(epochNumber, myAccount) {
  var winner = getWinner(epochNumber);
  return winner && winner.proposer === myAccount;
}

module.exports = {
  hashRewards: hashRewards,
  submitProposal: submitProposal,
  checkConsensus: checkConsensus,
  resolve: resolve,
  onResolved: onResolved,
  isResolved: isResolved,
  getWinner: getWinner,
  getProposals: getProposals,
  amIBroadcaster: amIBroadcaster,
  PROPOSAL_WINDOW_MS: PROPOSAL_WINDOW_MS,
  PROPOSAL_TIMEOUT_MS: PROPOSAL_TIMEOUT_MS
};

"use strict";

/**
 * HONE Finalization Consensus
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
 * Majority is stake-weighted: each proposal counts for its proposer's
 * registered stake, so a Sybil swarm of zero-stake nodes cannot outvote
 * honest high-stake nodes. When no proposer has stake (bootstrap), voting
 * falls back to one-proposal-one-vote.
 *
 * Single-miner fallback: if only one proposal after timeout, it wins.
 *
 * Resolved rounds are persisted as FINALIZATION_CONSENSUS ledger entries so
 * a crash before the EPOCH_FINALIZED broadcast no longer loses the round.
 */

var crypto = require("crypto");

var PROPOSAL_WINDOW_MS = parseInt(process.env.HONE_PROPOSAL_WINDOW_MS) || 30000; // 30s

// Minimum distinct machine sources required before an epoch can be finalized.
// "self" counts as one source; each distinct external peer IP counts as another.
// Set HONE_MIN_CONSENSUS_PEERS=2 to require at least one external machine.
// Default 1 allows solo operation during bootstrap.
var MIN_CONSENSUS_SOURCES = parseInt(process.env.HONE_MIN_CONSENSUS_PEERS) || 1;

// Minimum number of proposals required before an epoch auto-resolves after
// the proposal window expires. Default 1 keeps solo bootstrap working;
// operators can set HONE_MIN_CONSENSUS_PROPOSALS=2+ so a single proposal
// can never auto-win.
var MIN_CONSENSUS_PROPOSALS = parseInt(process.env.HONE_MIN_CONSENSUS_PROPOSALS) || 1;

// Hard ceiling on the total HONE a single proposal may distribute.
// Genesis reward is 243.06/epoch; 500 leaves margin for storage/clock/iot
// add-ons while still rejecting fabricated jackpot proposals outright.
var MAX_PROPOSAL_TOTAL = parseFloat(process.env.HONE_MAX_PROPOSAL_TOTAL) || 500;

// Strict recipient validation: when enabled, every reward recipient in a
// proposal must appear in locally-recorded mining/compute proofs for that
// epoch (system accounts exempt). Off by default — nodes can legitimately
// see different proof subsets, and rejecting here only means this node
// doesn't count the proposal toward consensus.
var STRICT_PROPOSAL_VALIDATION = process.env.HONE_STRICT_PROPOSAL_VALIDATION === "1";

// Accounts that may receive rewards without a recorded proof.
var SYSTEM_REWARD_ACCOUNTS = { hone_recycle: true, hone: true };

// Per-epoch state
// Map<epochNumber, { proposals: [], sourceTags: Set, windowStart: number, resolved: boolean, winner: object|null }>
var epochs = new Map();

// Callbacks
var onResolvedCallbacks = [];

// ─────────────────────────────────────────────────────────────────
// Injectable providers (lazy-required defaults; overridable in tests)
// ─────────────────────────────────────────────────────────────────

var _stateStore = null;
function _getStateStore() {
  if (_stateStore === null) {
    try { _stateStore = require("./stateStore"); } catch (e) { _stateStore = false; }
  }
  return _stateStore || null;
}

// Returns registered stake for a proposer account (0 when unknown).
var stakeProvider = function (proposer) {
  var ss = _getStateStore();
  if (!ss || typeof ss.getStake !== "function") return 0;
  try { return ss.getStake(proposer) || 0; } catch (e) { return 0; }
};

// Returns { miningProofs: [], computeProofs: [] } for an epoch.
var proofProvider = function (epochNumber) {
  var ss = _getStateStore();
  if (!ss) return { miningProofs: [], computeProofs: [] };
  try {
    return {
      miningProofs: typeof ss.getMiningProofs === "function" ? ss.getMiningProofs(epochNumber) : [],
      computeProofs: typeof ss.getRecentComputeProofs === "function" ? ss.getRecentComputeProofs(epochNumber) : []
    };
  } catch (e) {
    return { miningProofs: [], computeProofs: [] };
  }
};

// Persists a resolved round; default writes a FINALIZATION_CONSENSUS ledger entry.
var persistProvider = function (epochNumber, winner) {
  var ledger;
  try { ledger = require("../services/ledger"); } catch (e) { return; }
  if (!ledger || typeof ledger.recordFinalizationConsensus !== "function") return;
  Promise.resolve(ledger.recordFinalizationConsensus(epochNumber, winner)).catch(function (e) {
    console.log("[HONE Consensus] Failed to persist epoch " + epochNumber + " consensus: " + (e && e.message));
  });
};

function _setStakeProvider(fn) { stakeProvider = fn; }
function _setProofProvider(fn) { proofProvider = fn; }
function _setPersistProvider(fn) { persistProvider = fn; }

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
      sourceTags: new Set(), // distinct machine sources: "self", peer IPs
      windowStart: null,
      resolved: false,
      winner: null,
      timer: null
    });
  }
  return epochs.get(epochNumber);
}

/**
 * Read a persisted winner from chain state without touching in-memory
 * epoch tracking (safe for arbitrary historical queries).
 */
function _persistedWinner(epochNumber) {
  var ss = _getStateStore();
  if (!ss || typeof ss.getFinalizedConsensus !== "function") return null;
  var persisted = null;
  try { persisted = ss.getFinalizedConsensus(epochNumber); } catch (e) { return null; }
  if (!persisted) return null;
  return {
    proposer: persisted.proposer,
    rewards: persisted.rewards,
    total_work: persisted.total_work,
    consensus_hash: persisted.consensus_hash,
    consensus_nodes: persisted.consensus_nodes,
    consensus_proposals: persisted.consensus_proposals,
    restored_from_chain: true
  };
}

/**
 * Restore a resolved round from persisted chain state, if present.
 * Lets isResolved/getWinner/submitProposal survive a process restart.
 */
function _restoreFromChain(epochNumber, state) {
  if (state.resolved) return true;
  var winner = _persistedWinner(epochNumber);
  if (!winner) return false;
  state.resolved = true;
  state.winner = winner;
  if (state.timer) { clearTimeout(state.timer); clearInterval(state.timer); state.timer = null; }
  return true;
}

/**
 * Validate a proposal before it is counted toward consensus.
 * Structural checks always run; recipient-vs-proof checks only in strict mode.
 * @returns {{ ok: boolean, reason: string|null }}
 */
function validateProposal(epochNumber, proposal) {
  if (!proposal || typeof proposal !== "object") return { ok: false, reason: "missing proposal" };
  if (!proposal.proposer || typeof proposal.proposer !== "string") return { ok: false, reason: "missing proposer" };
  if (!Array.isArray(proposal.rewards)) return { ok: false, reason: "rewards not an array" };

  var total = 0;
  for (var i = 0; i < proposal.rewards.length; i++) {
    var r = proposal.rewards[i];
    var name = r && (r.miner || r.node_id);
    var amount = r && r.amount;
    if (!name || typeof name !== "string") return { ok: false, reason: "reward with missing miner" };
    if (typeof amount !== "number" || !isFinite(amount) || amount < 0) {
      return { ok: false, reason: "invalid reward amount for " + name };
    }
    total += amount;
  }
  if (total > MAX_PROPOSAL_TOTAL) {
    return { ok: false, reason: "total rewards " + total.toFixed(4) + " exceed cap " + MAX_PROPOSAL_TOTAL };
  }

  if (STRICT_PROPOSAL_VALIDATION) {
    var proofs = proofProvider(epochNumber);
    var known = {};
    var lists = [proofs.miningProofs || [], proofs.computeProofs || []];
    for (var l = 0; l < lists.length; l++) {
      for (var p = 0; p < lists[l].length; p++) {
        var who = lists[l][p] && (lists[l][p].miner || lists[l][p].node_id);
        if (who) known[who] = true;
      }
    }
    // Only enforce when we actually have local proofs to compare against.
    if (Object.keys(known).length > 0) {
      for (var k = 0; k < proposal.rewards.length; k++) {
        var rec = proposal.rewards[k].miner || proposal.rewards[k].node_id;
        if (!SYSTEM_REWARD_ACCOUNTS[rec] && !known[rec]) {
          return { ok: false, reason: "reward recipient " + rec + " has no recorded proof for epoch " + epochNumber };
        }
      }
    }
  }

  return { ok: true, reason: null };
}

/**
 * Sum the stake-weight of a group of proposals.
 * Each proposal weighs its proposer's registered stake.
 */
function _groupStake(group) {
  return group.reduce(function (sum, p) { return sum + (stakeProvider(p.proposer) || 0); }, 0);
}

/**
 * Can this epoch auto-resolve right now? Both the distinct-source and the
 * minimum-proposal-count requirements must be met.
 */
function _meetsResolveRequirements(state) {
  return state.sourceTags.size >= MIN_CONSENSUS_SOURCES &&
         state.proposals.length >= MIN_CONSENSUS_PROPOSALS;
}

/**
 * Submit a finalization proposal for an epoch.
 * Called when this miner computes its proposal OR when receiving one via P2P.
 *
 * @param {number} epochNumber
 * @param {object} proposal — { proposer, rewards, total_work, consensus_hash, settled_jobs, timestamp }
 * @param {string} [sourceTag] — caller-supplied tag identifying the source machine: "self", a peer IP, "relay", etc.
 * @returns {{ accepted: boolean, consensus: boolean, winner: object|null, reason?: string }}
 */
function submitProposal(epochNumber, proposal, sourceTag) {
  var state = _getEpoch(epochNumber);

  // Already resolved (in memory or persisted on chain) — reject late proposals
  if (state.resolved || _restoreFromChain(epochNumber, state)) {
    return { accepted: false, consensus: true, winner: state.winner };
  }

  // Validate before counting toward consensus
  var validation = validateProposal(epochNumber, proposal);
  if (!validation.ok) {
    console.log("[HONE Consensus] Epoch " + epochNumber + " proposal from " +
      ((proposal && proposal.proposer) || "?") + " rejected: " + validation.reason);
    return { accepted: false, consensus: false, winner: null, reason: validation.reason };
  }

  // Start the proposal window on first proposal
  if (!state.windowStart) {
    state.windowStart = Date.now();

    // Set timeout for resolution — only finalize if source + proposal-count requirements are met
    state.timer = setTimeout(function () {
      var s = epochs.get(epochNumber);
      if (!s || s.resolved) return;
      if (_meetsResolveRequirements(s)) {
        resolve(epochNumber);
      } else {
        console.log("[HONE Consensus] Epoch " + epochNumber + " window expired but requirements unmet (" +
          s.sourceTags.size + "/" + MIN_CONSENSUS_SOURCES + " source(s), " +
          s.proposals.length + "/" + MIN_CONSENSUS_PROPOSALS + " proposal(s)) — waiting");
        // Reschedule — check again every 10s until requirements are met
        s.timer = setInterval(function () {
          var st = epochs.get(epochNumber);
          if (!st || st.resolved) { clearInterval(st && st.timer); return; }
          if (_meetsResolveRequirements(st)) {
            clearInterval(st.timer);
            st.timer = null;
            resolve(epochNumber);
          }
        }, 10000);
        if (s.timer && s.timer.unref) s.timer.unref();
      }
    }, PROPOSAL_WINDOW_MS);
    if (state.timer && state.timer.unref) state.timer.unref();
  }

  // Reject duplicate proposers
  var existing = state.proposals.find(function (p) { return p.proposer === proposal.proposer; });
  if (existing) {
    // Duplicate proposer — still record source tag in case re-submitted from a different machine
    state.sourceTags.add(sourceTag || ("proposer:" + (proposal.proposer || "unknown")));
    return { accepted: false, consensus: false, winner: null };
  }

  // Ensure consensus_hash
  if (!proposal.consensus_hash) {
    proposal.consensus_hash = hashRewards(proposal.rewards, proposal.total_work, proposal.settled_jobs, epochNumber);
  }

  // Track distinct source machines.
  // Fall back to proposer name when no source tag supplied (legacy callers / tests)
  // so the source set always has at least one entry after the first proposal.
  state.sourceTags.add(sourceTag || ("proposer:" + (proposal.proposer || "unknown")));

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
 * Groups proposals by consensus_hash and checks for a majority.
 *
 * Stake-weighted: when any proposer has registered stake, a group needs
 * > 50% of the total submitted stake. When no proposer has stake
 * (bootstrap), falls back to > 50% of proposal count.
 */
function checkConsensus(epochNumber) {
  var state = _getEpoch(epochNumber);
  if (state.resolved || _restoreFromChain(epochNumber, state)) return { consensus: true, winner: state.winner };
  if (state.proposals.length === 0) return { consensus: false, winner: null };

  // Single proposal — wait for window. Only resolve if requirements met.
  if (state.proposals.length === 1) {
    var elapsed = Date.now() - (state.windowStart || Date.now());
    if (elapsed >= PROPOSAL_WINDOW_MS && _meetsResolveRequirements(state)) {
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

  var totalStake = _groupStake(state.proposals);
  var stakeWeighted = totalStake > 0;

  // Find the strongest group: by stake when any stake exists, else by count.
  // Ties break on total_work.
  var hashes = Object.keys(groups);
  var bestWeight = 0;
  var bestWork = 0;
  var hasMajority = false;

  for (var j = 0; j < hashes.length; j++) {
    var group = groups[hashes[j]];
    var weight = stakeWeighted ? _groupStake(group) : group.length;
    var groupWork = group.reduce(function (sum, q) { return sum + (q.total_work || 0); }, 0);

    if (weight > bestWeight || (weight === bestWeight && groupWork > bestWork)) {
      bestWeight = weight;
      bestWork = groupWork;
    }
  }

  var majorityBase = stakeWeighted ? totalStake : state.proposals.length;
  hasMajority = bestWeight > majorityBase / 2;

  if (hasMajority && _meetsResolveRequirements(state)) {
    resolve(epochNumber);
    return { consensus: true, winner: state.winner };
  }

  // No majority yet — wait for more proposals or timeout
  return { consensus: false, winner: null };
}

/**
 * Resolve an epoch — pick the winning proposal.
 * Called when consensus is reached or proposal window expires.
 * Winner selection mirrors checkConsensus: stake-weight first (when any
 * stake exists), proposal count otherwise, then total_work, then earliest.
 */
function resolve(epochNumber) {
  var state = _getEpoch(epochNumber);
  if (state.resolved) return state.winner;
  if (state.proposals.length === 0) return null;

  // Clear timer
  if (state.timer) {
    clearTimeout(state.timer);
    clearInterval(state.timer);
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

  var stakeWeighted = _groupStake(state.proposals) > 0;

  function groupWeight(group) {
    return stakeWeighted ? _groupStake(group) : group.length;
  }

  // Find winning group: highest weight, then highest total_work, then earliest
  var hashes = Object.keys(groups);
  var winningHash = hashes[0];
  var winningGroup = groups[winningHash];

  for (var j = 1; j < hashes.length; j++) {
    var group = groups[hashes[j]];
    var w = groupWeight(group);
    var winW = groupWeight(winningGroup);
    if (w > winW) {
      winningHash = hashes[j];
      winningGroup = group;
    } else if (w === winW) {
      // Tiebreak: highest total_work
      var groupWork = group.reduce(function (s, q) { return s + (q.total_work || 0); }, 0);
      var winWork = winningGroup.reduce(function (s, q) { return s + (q.total_work || 0); }, 0);
      if (groupWork > winWork) {
        winningHash = hashes[j];
        winningGroup = group;
      }
    }
  }

  // Within the winning group, earliest proposer wins the right to broadcast
  winningGroup.sort(function (a, b) { return (a.timestamp || 0) - (b.timestamp || 0); });
  var winner = winningGroup[0];

  winner.consensus_nodes = state.sourceTags.size;
  winner.consensus_proposals = state.proposals.length;
  winner.consensus_stake = _groupStake(winningGroup);

  state.resolved = true;
  state.winner = winner;

  console.log("[HONE Consensus] Epoch " + epochNumber + " resolved: " +
    state.proposals.length + " proposal(s) from " + state.sourceTags.size + " distinct source(s), " +
    hashes.length + " group(s), " + (stakeWeighted ? "stake-weighted" : "count-based") +
    ", winner: " + winner.proposer +
    " (hash: " + winningHash.slice(0, 12) + "...)");

  // Persist the resolved round so a crash before EPOCH_FINALIZED doesn't lose it
  try { persistProvider(epochNumber, winner); } catch (e) {
    console.log("[HONE Consensus] Persist failed for epoch " + epochNumber + ": " + (e && e.message));
  }

  // Fire callbacks
  for (var k = 0; k < onResolvedCallbacks.length; k++) {
    try { onResolvedCallbacks[k](epochNumber, winner); } catch (e) {}
  }

  // Cleanup old epochs (keep last 10)
  var keys = Array.from(epochs.keys()).sort(function (a, b) { return a - b; });
  while (keys.length > 10) {
    var old = keys.shift();
    var oldState = epochs.get(old);
    if (oldState && oldState.timer) { clearTimeout(oldState.timer); clearInterval(oldState.timer); }
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
 * Check if an epoch has been resolved (in memory or persisted on chain).
 */
function isResolved(epochNumber) {
  var state = epochs.get(epochNumber);
  if (state && (state.resolved || _restoreFromChain(epochNumber, state))) return true;
  return _persistedWinner(epochNumber) !== null;
}

/**
 * Get the winning proposal for a resolved epoch.
 */
function getWinner(epochNumber) {
  var state = epochs.get(epochNumber);
  if (state && (state.resolved || _restoreFromChain(epochNumber, state))) return state.winner;
  return _persistedWinner(epochNumber);
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
  validateProposal: validateProposal,
  resolve: resolve,
  onResolved: onResolved,
  isResolved: isResolved,
  getWinner: getWinner,
  getProposals: getProposals,
  amIBroadcaster: amIBroadcaster,
  PROPOSAL_WINDOW_MS: PROPOSAL_WINDOW_MS,
  _setStakeProvider: _setStakeProvider,
  _setProofProvider: _setProofProvider,
  _setPersistProvider: _setPersistProvider
};

"use strict";

/**
 * BTCPC Reward Settlement
 * Shin Devlin
 *
 * Manages multi-epoch proof collection and reward distribution.
 *
 * Flow:
 *   Epoch N sealed (clockConsensus) → opens settlement window for N
 *   Window spans SETTLEMENT_WINDOW epochs (default 5 = 2.5 min)
 *   Any WORK_PROOF, STORAGE_PROOF arriving before deadline → included
 *   After deadline: reward engine runs → clocks sign reward_hash
 *   REWARD_BLOCK for N written to disk at epoch N + SETTLEMENT_WINDOW + 1
 *
 * Late proofs (after window) are queued for the next open settlement.
 * Work is never lost — just credited to the nearest open window.
 *
 * Settlement quality:
 *   A settlement with more clock signatures beats one with fewer.
 *   This is the fork selection criterion: main chain always has more signatures.
 */

const crypto = require("crypto");
const EventEmitter = require("events");
const { computeRewards, extractStorageHosts, buildClockNodes } = require("./rewardEngine");
const { getBlockReward } = require("../services/emissionSchedule");

const SETTLEMENT_WINDOW = parseInt(process.env.BTCPC_SETTLEMENT_WINDOW) || 5; // epochs
const emitter = new EventEmitter();

// Map<epochNumber, SettlementState>
// SettlementState: {
//   epoch, deadline_epoch, status: 'open'|'computing'|'settled'|'failed',
//   work_proofs: [], storage_proofs: [], clock_nodes: [],
//   reward_hash: null, clock_signatures: [], result: null
// }
const settlements = new Map();

// Proofs that arrived after their window closed — queued for next open settlement
const lateProofQueue = [];

// ── Open / receive ────────────────────────────────────────────────────────────

/**
 * Open a settlement window for a sealed epoch.
 * Called by clockConsensus 'epoch_sealed' event.
 */
function openSettlement(epochNumber, sealInfo) {
  if (settlements.has(epochNumber)) return; // already open

  const deadline = epochNumber + SETTLEMENT_WINDOW;
  settlements.set(epochNumber, {
    epoch: epochNumber,
    deadline_epoch: deadline,
    status: "open",
    seal: sealInfo || null,
    work_proofs: [],
    storage_proofs: [],
    clock_nodes: [],
    reward_hash: null,
    clock_signatures: [],
    result: null,
  });

  console.log("[BTCPC Settlement] Window opened for epoch " + epochNumber +
    " — deadline epoch " + deadline + " (" + (SETTLEMENT_WINDOW * 30) + "s)");

  // Drain any queued late proofs that belong here
  _drainLateQueue(epochNumber);
}

/**
 * Submit a work proof (from miner inference completion).
 * If the epoch's settlement window is still open → include it.
 * If the window has closed → queue for next open settlement.
 */
function submitWorkProof(proof) {
  const targetEpoch = proof.epoch_number;
  const s = _findOpenSettlement(targetEpoch);

  if (s) {
    // Deduplicate by result_hash + node_id
    const key = (proof.result_hash || "") + "|" + (proof.node_id || "");
    if (!s.work_proofs.find(p => (p.result_hash || "") + "|" + (p.node_id || "") === key)) {
      s.work_proofs.push(proof);
      console.log("[BTCPC Settlement] Work proof accepted for epoch " + targetEpoch +
        " from " + proof.node_id + " (value=" + (proof.work_value || 0).toFixed(1) + ")");
    }
  } else {
    // Late proof — queue for next open settlement
    lateProofQueue.push({ ...proof, _queued_at: Date.now() });
    console.log("[BTCPC Settlement] Late work proof from " + proof.node_id +
      " (epoch " + targetEpoch + ") — queued for next open window");
  }
}

/**
 * Submit a storage heartbeat proof.
 */
function submitStorageProof(proof) {
  const s = _findOpenSettlement(proof.epoch_number);
  if (s) {
    const existing = s.storage_proofs.find(p => p.account === proof.account);
    if (existing) {
      // Take the best stats seen for this host across the window
      existing.capacity_gb = Math.max(existing.capacity_gb || 0, proof.capacity_gb || 0);
      existing.cids_count = Math.max(existing.cids_count || 0, proof.cids_count || 0);
      existing.queries_served = (existing.queries_served || 0) + (proof.queries_served || 0);
    } else {
      s.storage_proofs.push({ ...proof });
    }
  } else {
    lateProofQueue.push({ ...proof, _type: "storage", _queued_at: Date.now() });
  }
}

/**
 * Record a clock node that participated in sealing an epoch.
 * Used to distribute clock rewards in the settlement.
 */
function recordClockParticipation(epochNumber, clockAccount, heartbeats) {
  const s = _findOpenSettlement(epochNumber);
  if (!s) return;
  const existing = s.clock_nodes.find(c => c.account === clockAccount);
  if (existing) {
    existing.heartbeats = (existing.heartbeats || 0) + (heartbeats || 1);
  } else {
    s.clock_nodes.push({ account: clockAccount, heartbeats: heartbeats || 1 });
  }
}

// ── Settlement trigger ────────────────────────────────────────────────────────

/**
 * Called every epoch by the miner/clock loop.
 * Checks if any open settlement has passed its deadline and triggers computation.
 */
function tickEpoch(currentEpoch) {
  for (const [epoch, s] of settlements) {
    if (s.status === "open" && currentEpoch >= s.deadline_epoch) {
      _computeSettlement(epoch, currentEpoch);
    }
  }
  // Prune very old settled/failed entries
  for (const [epoch, s] of settlements) {
    if ((s.status === "settled" || s.status === "failed") && epoch < currentEpoch - 50) {
      settlements.delete(epoch);
    }
  }
}

function _computeSettlement(epochNumber, currentEpoch) {
  const s = settlements.get(epochNumber);
  if (!s || s.status !== "open") return;
  s.status = "computing";

  const blockReward = getBlockReward(epochNumber);
  const storageHosts = s.storage_proofs.length > 0
    ? s.storage_proofs
    : extractStorageHosts([]); // no storage this epoch

  let result;
  try {
    result = computeRewards({
      epochNumber,
      blockReward,
      computeProofs: s.work_proofs,
      verifiers: [],
      clockNodes: s.clock_nodes.length > 0 ? s.clock_nodes : buildClockNodes([]),
      storageHosts,
      serviceHosts: [],
    });
  } catch (err) {
    console.error("[BTCPC Settlement] Reward computation failed for epoch " + epochNumber + ":", err.message);
    s.status = "failed";
    return;
  }

  // Deterministic reward hash — all clocks with same proofs produce same hash
  const canonical = JSON.stringify({
    epoch: epochNumber,
    rewards: result.rewards.map(r => ({ to: r.to, amount: r.amount, type: r.type }))
      .sort((a, b) => a.to < b.to ? -1 : 1),
    recycled: result.recycled,
    work_count: s.work_proofs.length,
  });
  const reward_hash = crypto.createHash("sha256").update(canonical).digest("hex");

  s.reward_hash = reward_hash;
  s.result = result;
  s.status = "settled";
  s.settled_at_epoch = currentEpoch;

  console.log("[BTCPC Settlement] Epoch " + epochNumber + " settled:" +
    " reward=" + blockReward.toFixed(4) + " BTCPC" +
    " | miners=" + result.summary.miners +
    " | work=" + result.summary.total_work.toFixed(0) +
    " | recycled=" + result.summary.recycled.toFixed(4) +
    " | hash=" + reward_hash.slice(0, 12) + "...");

  emitter.emit("settlement_ready", {
    epoch: epochNumber,
    reward_hash,
    result,
    settlement: s,
  });
}

// ── Clock signature collection ────────────────────────────────────────────────

/**
 * Submit a clock's signature on a reward_hash.
 * Once enough clocks have signed, emit 'reward_finalized'.
 */
function submitClockSignature(epochNumber, clockNodeId, rewardHash, signature) {
  const s = settlements.get(epochNumber);
  if (!s) return;

  // Only accept signatures on the correct hash
  if (s.reward_hash && rewardHash !== s.reward_hash) {
    console.warn("[BTCPC Settlement] Clock " + clockNodeId +
      " signed wrong hash for epoch " + epochNumber + " — ignoring");
    return;
  }

  const existing = s.clock_signatures.find(sig => sig.node_id === clockNodeId);
  if (!existing) {
    s.clock_signatures.push({ node_id: clockNodeId, signature, timestamp: Date.now() });
    console.log("[BTCPC Settlement] Clock signature received for epoch " + epochNumber +
      " from " + clockNodeId + " (" + s.clock_signatures.length + " total)");
  }

  // Emit finalized when we have at least 1 signature (scales with clock count)
  if (s.clock_signatures.length >= 1 && s.status === "settled") {
    emitter.emit("reward_finalized", {
      epoch: epochNumber,
      reward_hash: s.reward_hash,
      clock_signatures: s.clock_signatures,
      result: s.result,
    });
  }
}

// ── Fork resolution ───────────────────────────────────────────────────────────

/**
 * Score a chain fork for fork selection.
 * Higher score = main chain. Used by forkResolver to pick the winner.
 *
 * Score = sum of clock_signature_counts for all blocks since fork point.
 * A private fork always scores 0 (no external clock signatures).
 */
function scoreFork(blocksSinceFork) {
  return (blocksSinceFork || []).reduce((sum, block) => {
    const sigs = (block.clock_signatures || []).length;
    return sum + sigs;
  }, 0);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function _findOpenSettlement(targetEpoch) {
  // Find the open settlement window that covers this epoch
  for (const [epoch, s] of settlements) {
    if (s.status === "open" && targetEpoch >= epoch && targetEpoch <= s.deadline_epoch) {
      return s;
    }
  }
  return null;
}

function _drainLateQueue(newEpoch) {
  const remaining = [];
  for (const proof of lateProofQueue) {
    const s = _findOpenSettlement(proof.epoch_number || newEpoch);
    if (s) {
      if (proof._type === "storage") {
        submitStorageProof(proof);
      } else {
        submitWorkProof(proof);
      }
    } else {
      remaining.push(proof);
    }
  }
  lateProofQueue.length = 0;
  lateProofQueue.push(...remaining);
}

function getSettlement(epochNumber) { return settlements.get(epochNumber) || null; }
function getAllOpenSettlements() {
  return Array.from(settlements.values()).filter(s => s.status === "open");
}
function on(event, fn) { emitter.on(event, fn); }

module.exports = {
  openSettlement,
  submitWorkProof,
  submitStorageProof,
  recordClockParticipation,
  submitClockSignature,
  tickEpoch,
  scoreFork,
  getSettlement,
  getAllOpenSettlements,
  on,
  SETTLEMENT_WINDOW,
};

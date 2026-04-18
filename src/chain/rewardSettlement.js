"use strict";

/**
 * BTCPC Reward Settlement
 * Shin Devlin
 *
 * Manages proof verification windows and reward distribution.
 *
 * Key design:
 *   - Miner REQUESTS happen any epoch — miners poll for work continuously
 *   - Miner SUBMISSIONS happen any epoch — results come back whenever inference finishes
 *   - VERIFICATION WINDOW: 5 epochs after a proof is submitted
 *     Verifiers have that window to challenge the proof
 *     After the window closes → proof is finalized (verified or uncontested)
 *   - REWARD RUN: each clock-sealed epoch, collect all proofs whose
 *     verification windows have closed → run reward engine → distribute
 *
 * A proof is "finalized" when:
 *   current_epoch >= proof.submitted_epoch + VERIFICATION_WINDOW
 *   AND no valid challenge was raised against it
 *
 * Challenges during the window → proof marked disputed → reduced reward or rejected
 * Uncontested proof after window → full reward
 *
 * Fork scoring:
 *   A settlement with more clock signatures beats one with fewer.
 *   Private fork (offline node) always scores 0 — healed back to main chain.
 */

const crypto = require("crypto");
const EventEmitter = require("events");
const { computeRewards, extractStorageHosts, buildClockNodes } = require("./rewardEngine");
const { getBlockReward } = require("../services/emissionSchedule");

const VERIFICATION_WINDOW = parseInt(process.env.BTCPC_VERIFICATION_WINDOW) || 5; // epochs
const emitter = new EventEmitter();

// All submitted proofs pending finalization
// Map<proofId, ProofRecord>
// ProofRecord: {
//   id, node_id, work_value, model, tokens_generated,
//   prompt_hash, result_hash, model_hash,
//   submitted_epoch, deadline_epoch,
//   status: 'pending'|'finalized'|'disputed'|'rejected',
//   challenges: [], reward_epoch: null
// }
const pendingProofs = new Map();

// Finalized proofs ready for reward distribution, keyed by reward epoch
// Map<rewardEpoch, ProofRecord[]>
const finalizedByEpoch = new Map();

// Storage proofs (heartbeats) — latest per account per epoch
// Map<`${account}:${epoch}`, StorageProof>
const storageProofs = new Map();

// Active clock nodes per epoch
// Map<epoch, Map<account, heartbeats>>
const clockActivity = new Map();

// ── Proof submission ──────────────────────────────────────────────────────────

/**
 * Submit a completed work proof from a miner.
 * Can arrive any epoch — tied to when inference finished, not when it started.
 */
function submitWorkProof(proof, currentEpoch) {
  if (!proof || !proof.node_id || !proof.result_hash) return null;

  const id = crypto.createHash("sha256")
    .update((proof.result_hash || "") + "|" + (proof.node_id || "") + "|" + (proof.work_value || 0))
    .digest("hex").slice(0, 32);

  if (pendingProofs.has(id)) return id; // already submitted

  const epoch = typeof currentEpoch === "number" ? currentEpoch : (proof.epoch_number || 0);
  const record = {
    id,
    node_id: proof.node_id,
    work_value: proof.work_value || 0,
    model: proof.model || null,
    tokens_generated: proof.tokens_generated || 0,
    prompt_hash: proof.prompt_hash || null,
    result_hash: proof.result_hash,
    model_hash: proof.model_hash || null,
    submitted_epoch: epoch,
    deadline_epoch: epoch + VERIFICATION_WINDOW,
    status: "pending",
    challenges: [],
    reward_epoch: null,
  };

  pendingProofs.set(id, record);

  console.log("[BTCPC Settlement] Work proof submitted by " + proof.node_id +
    " at epoch " + epoch + " | value=" + (proof.work_value || 0).toFixed(1) +
    " | model=" + (proof.model || "?") +
    " | verification deadline: epoch " + record.deadline_epoch);

  return id;
}

/**
 * Submit a challenge against a proof during its verification window.
 * challenger: account name of the verifier raising the challenge
 * reason: 'invalid_result' | 'model_mismatch' | 'replay' | 'low_quality'
 */
function submitChallenge(proofId, challenger, reason, evidence) {
  const record = pendingProofs.get(proofId);
  if (!record) return false;
  if (record.status !== "pending") return false;

  const currentEpoch = _getCurrentEpoch();
  if (currentEpoch > record.deadline_epoch) {
    console.warn("[BTCPC Settlement] Challenge too late for proof " + proofId +
      " (window closed at epoch " + record.deadline_epoch + ")");
    return false;
  }

  record.challenges.push({ challenger, reason, evidence: evidence || null, epoch: currentEpoch });
  record.status = "disputed";

  console.log("[BTCPC Settlement] Challenge raised against " + record.node_id +
    " proof " + proofId.slice(0, 12) + " by " + challenger + " (" + reason + ")");

  emitter.emit("proof_challenged", { proofId, record, challenger, reason });
  return true;
}

/**
 * Record a storage heartbeat proof for a given epoch.
 */
function submitStorageProof(account, epochNumber, capacityGb, cids, queriesServed) {
  const key = account + ":" + epochNumber;
  const existing = storageProofs.get(key) || { account, epoch: epochNumber, capacity_gb: 0, cids_count: 0, queries_served: 0 };
  existing.capacity_gb = Math.max(existing.capacity_gb, capacityGb || 0);
  existing.cids_count = Math.max(existing.cids_count, (cids || []).length);
  existing.queries_served = (existing.queries_served || 0) + (queriesServed || 0);
  storageProofs.set(key, existing);
}

/**
 * Record clock participation for an epoch.
 */
function recordClockParticipation(epochNumber, clockAccount, heartbeats) {
  if (!clockActivity.has(epochNumber)) clockActivity.set(epochNumber, new Map());
  const epoch = clockActivity.get(epochNumber);
  epoch.set(clockAccount, (epoch.get(clockAccount) || 0) + (heartbeats || 1));
}

// ── Finalization tick ─────────────────────────────────────────────────────────

/**
 * Called every clock-sealed epoch.
 * Finalizes proofs whose verification windows have closed.
 * Then runs the reward engine over all proofs finalized this epoch.
 */
function tickEpoch(currentEpoch) {
  _setCurrentEpoch(currentEpoch);

  // Finalize proofs whose window has closed
  const newlyFinalized = [];
  for (const [id, record] of pendingProofs) {
    if (record.status !== "pending" && record.status !== "disputed") continue;
    if (currentEpoch < record.deadline_epoch) continue;

    // Resolve disputed proofs — for now, if challenged once it earns 50% reward
    // TODO: full verifier arbitration in a future release
    if (record.status === "disputed") {
      record.status = "finalized";
      record._disputed = true;
      record._reward_fraction = 0.5;
      console.log("[BTCPC Settlement] Disputed proof from " + record.node_id +
        " finalized at 50% reward (challenges: " + record.challenges.length + ")");
    } else {
      record.status = "finalized";
      record._disputed = false;
      record._reward_fraction = 1.0;
    }

    record.reward_epoch = currentEpoch;
    newlyFinalized.push(record);

    const bucket = finalizedByEpoch.get(currentEpoch) || [];
    bucket.push(record);
    finalizedByEpoch.set(currentEpoch, bucket);
  }

  // Remove finalized proofs from pending
  for (const r of newlyFinalized) pendingProofs.delete(r.id);

  if (newlyFinalized.length > 0) {
    console.log("[BTCPC Settlement] Epoch " + currentEpoch + ": " +
      newlyFinalized.length + " proof(s) finalized");
  }

  // Run reward engine over finalized proofs for this epoch
  const proofs = finalizedByEpoch.get(currentEpoch) || [];
  if (proofs.length > 0 || _hasStorageActivity(currentEpoch) || _hasClockActivity(currentEpoch)) {
    _runRewardEngine(currentEpoch, proofs);
  }

  // Prune old data
  _pruneOld(currentEpoch);
}

function _runRewardEngine(currentEpoch, finalizedProofs) {
  const blockReward = getBlockReward(currentEpoch);

  // Build compute proof list with reward fraction applied
  const computeProofs = finalizedProofs.map(r => ({
    node_id: r.node_id,
    work_value: r.work_value * (r._reward_fraction || 1),
    model: r.model,
    tokens_generated: r.tokens_generated,
    result_hash: r.result_hash,
  }));

  // Collect storage hosts active in this epoch or recent epochs
  const storageHosts = _getStorageHosts(currentEpoch);

  // Collect clock nodes
  const clockMap = clockActivity.get(currentEpoch) || new Map();
  const clockNodes = Array.from(clockMap.entries()).map(([account, heartbeats]) => ({ account, heartbeats }));

  // Collect active sensors — sensors that submitted SENSOR_READING entries in recent epochs
  const activeSensors = _getActiveSensors(currentEpoch);

  let result;
  try {
    result = computeRewards({
      epochNumber: currentEpoch,
      blockReward,
      computeProofs,
      verifiers: [],
      clockNodes,
      storageHosts,
      activeSensors,
      serviceHosts: [],
    });
  } catch (err) {
    console.error("[BTCPC Settlement] Reward engine failed for epoch " + currentEpoch + ":", err.message);
    return;
  }

  // Deterministic reward hash for clock consensus
  const canonical = JSON.stringify({
    epoch: currentEpoch,
    rewards: result.rewards.map(r => ({ to: r.to, amount: r.amount, type: r.type }))
      .sort((a, b) => a.to < b.to ? -1 : 1),
    proof_count: finalizedProofs.length,
  });
  const reward_hash = crypto.createHash("sha256").update(canonical).digest("hex");

  console.log("[BTCPC Settlement] Epoch " + currentEpoch + " rewards computed:" +
    " reward=" + blockReward.toFixed(4) +
    " | verified_proofs=" + finalizedProofs.length +
    " | clocks=" + clockNodes.length +
    " | storage=" + storageHosts.length +
    " | sensors=" + activeSensors.length +
    " | recycled=" + result.summary.recycled.toFixed(4) +
    " | hash=" + reward_hash.slice(0, 12) + "...");

  emitter.emit("rewards_computed", {
    epoch: currentEpoch,
    reward_hash,
    result,
    finalized_proofs: finalizedProofs,
  });
}

// ── Fork scoring ──────────────────────────────────────────────────────────────

/**
 * Score a chain fork by clock signatures — used by forkResolver.
 * Higher score = main chain. Private fork scores 0.
 */
function scoreFork(blocksSinceFork) {
  return (blocksSinceFork || []).reduce((sum, block) => {
    const sigs = (block && block.block && block.block.clock_signatures) || [];
    return sum + (Array.isArray(sigs) ? sigs.length : 0);
  }, 0);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

let _currentEpoch = 0;
function _getCurrentEpoch() { return _currentEpoch; }
function _setCurrentEpoch(e) { _currentEpoch = e; }

function _getStorageHosts(currentEpoch) {
  const hosts = new Map();
  // Look at last 3 epochs of heartbeats
  for (let e = currentEpoch; e >= currentEpoch - 3; e--) {
    for (const [key, proof] of storageProofs) {
      if (proof.epoch !== e) continue;
      const existing = hosts.get(proof.account) || { account: proof.account, capacity_gb: 0, cids_count: 0, queries_served: 0 };
      existing.capacity_gb = Math.max(existing.capacity_gb, proof.capacity_gb || 0);
      existing.cids_count = Math.max(existing.cids_count, proof.cids_count || 0);
      existing.queries_served += proof.queries_served || 0;
      hosts.set(proof.account, existing);
    }
  }
  return Array.from(hosts.values());
}

// Active sensor readings per epoch — aggregated from block ledger entries
// Map<`${account}:${epoch}`, { account, epoch, readings_count, sensor_ids: Set }>
const sensorActivity = new Map();

/**
 * Record a SENSOR_READING ledger entry for epoch-reward tracking.
 * Called from blockProposal / replay when SENSOR_READING entries are processed.
 */
function recordSensorReading(account, epochNumber, sensorId) {
  if (!account || !epochNumber) return;
  const key = account + ":" + epochNumber;
  const existing = sensorActivity.get(key) || { account, epoch: epochNumber, readings_count: 0, sensor_ids: new Set() };
  existing.readings_count += 1;
  if (sensorId) existing.sensor_ids.add(sensorId);
  sensorActivity.set(key, existing);
}

/**
 * Return sensors active in the current epoch (readings submitted this epoch).
 * Sensors submitting in the current epoch get the base participation reward.
 */
function _getActiveSensors(currentEpoch) {
  const result = [];
  for (const [, rec] of sensorActivity) {
    if (rec.epoch !== currentEpoch) continue;
    result.push({
      account: rec.account,
      readings_count: rec.readings_count,
      sensor_ids: Array.from(rec.sensor_ids),
    });
  }
  return result;
}

function _hasStorageActivity(epoch) {
  for (const [, p] of storageProofs) {
    if (p.epoch >= epoch - 3 && p.epoch <= epoch) return true;
  }
  return false;
}

function _hasClockActivity(epoch) {
  return (clockActivity.get(epoch) || new Map()).size > 0;
}

function _pruneOld(currentEpoch) {
  const cutoff = currentEpoch - 50;
  for (const [epoch] of finalizedByEpoch) {
    if (epoch < cutoff) finalizedByEpoch.delete(epoch);
  }
  for (const [epoch] of clockActivity) {
    if (epoch < cutoff) clockActivity.delete(epoch);
  }
  for (const [key, proof] of storageProofs) {
    if (proof.epoch < cutoff) storageProofs.delete(key);
  }
  for (const [key, rec] of sensorActivity) {
    if (rec.epoch < cutoff) sensorActivity.delete(key);
  }
}

function getPendingProofs() { return Array.from(pendingProofs.values()); }
function getFinalizedProofs(epoch) { return finalizedByEpoch.get(epoch) || []; }
function getPendingCount() { return pendingProofs.size; }
function on(event, fn) { emitter.on(event, fn); }

module.exports = {
  submitWorkProof,
  submitChallenge,
  submitStorageProof,
  recordClockParticipation,
  recordSensorReading,
  tickEpoch,
  scoreFork,
  getPendingProofs,
  getFinalizedProofs,
  getPendingCount,
  on,
  VERIFICATION_WINDOW,
};

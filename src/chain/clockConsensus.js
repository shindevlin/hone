"use strict";

/**
 * BTCPC Clock Consensus
 * Shin Devlin
 *
 * Manages the dynamic clock quorum for epoch sealing.
 *
 * Design:
 *   - All clock nodes broadcast EPOCH_SEAL { epoch, timestamp, seal_hash, node_id }
 *   - Collect seals for a short window (SEAL_COLLECT_MS)
 *   - Compute median timestamp across received seals
 *   - Discard any seal whose timestamp deviates > OUTLIER_EPOCH_TOLERANCE epochs from median
 *   - Remaining cluster = quorum; must be >= simple majority of non-outlier clocks
 *   - Bootstrap (1 clock, internet connected): self-sign allowed
 *   - Isolated node (no external peers for > ISOLATION_EPOCH_THRESHOLD epochs):
 *     downgrade to observer — no sealing, no block production
 *
 * Clock quality score:
 *   Accumulated across epochs. Clocks that consistently agree with quorum score up.
 *   Persistent outliers score down and are eventually ignored automatically.
 */

const crypto = require("crypto");
const EventEmitter = require("events");

const SEAL_COLLECT_MS = parseInt(process.env.BTCPC_SEAL_COLLECT_MS) || 5000;
const OUTLIER_EPOCH_TOLERANCE = 2; // epochs (60s) max deviation from median
const EPOCH_MS = 30000;
const ISOLATION_EPOCH_THRESHOLD = 3; // epochs without external peer → observer mode
const MIN_QUORUM_FRACTION = 0.51; // simple majority of non-outlier clocks

// ── State ────────────────────────────────────────────────────────────────────

const emitter = new EventEmitter();

// Map<epochNumber, { seals: [], collectTimer, resolved, winner }>
const epochState = new Map();

// Map<nodeId, { score, last_seen, outlier_count, total_seals }>
const clockScores = new Map();

// Connectivity state
let externalPeerCount = 0;
let lastExternalPeerEpoch = 0;
let currentEpoch = 0;
let observerMode = false;

// ── Connectivity ─────────────────────────────────────────────────────────────

/**
 * Called by p2p/network when peer connections change.
 * Only non-localhost peers count as "external".
 */
function updatePeerConnectivity(peers) {
  const external = (peers || []).filter(p => {
    const addr = (p.address || "");
    return !addr.includes("localhost") && !addr.includes("127.0.0.1") && !addr.includes("::1");
  });
  externalPeerCount = external.length;
  if (externalPeerCount > 0) lastExternalPeerEpoch = currentEpoch;

  const isolated = (currentEpoch - lastExternalPeerEpoch) > ISOLATION_EPOCH_THRESHOLD;
  if (isolated !== observerMode) {
    observerMode = isolated;
    if (observerMode) {
      console.log("[BTCPC Clock] No external peers for " + ISOLATION_EPOCH_THRESHOLD +
        " epochs — downgrading to observer mode. Will not produce seals.");
    } else {
      console.log("[BTCPC Clock] External peers restored — resuming seal production.");
    }
  }
}

function isObserver() { return observerMode; }
function getExternalPeerCount() { return externalPeerCount; }

// ── Seal collection ───────────────────────────────────────────────────────────

/**
 * Submit a received EPOCH_SEAL message into the collection window.
 * Call this when a EPOCH_SEAL P2P message arrives, and also for our own seal.
 */
function receiveSeal(seal) {
  const epoch = seal.epoch_number;
  if (typeof epoch !== "number") return;

  if (!epochState.has(epoch)) {
    epochState.set(epoch, { seals: [], collectTimer: null, resolved: false, winner: null });

    // Start collection window — resolve after SEAL_COLLECT_MS
    const state = epochState.get(epoch);
    state.collectTimer = setTimeout(() => {
      _resolveEpoch(epoch);
    }, SEAL_COLLECT_MS);
  }

  const state = epochState.get(epoch);
  if (state.resolved) return;

  // Deduplicate by node_id
  const existing = state.seals.find(s => s.node_id === seal.node_id);
  if (!existing) {
    state.seals.push({
      node_id: seal.node_id,
      timestamp: seal.timestamp,
      seal_hash: seal.seal_hash,
      signature: seal.signature || null,
    });
  }
}

/**
 * Build and submit our own seal for an epoch (called by clock node at epoch boundary).
 * Returns null if in observer mode.
 */
function produceSeal(epochNumber, nodeId, signingFn) {
  currentEpoch = epochNumber;
  updatePeerConnectivity(_getPeers());

  if (observerMode) {
    console.log("[BTCPC Clock] Observer mode — skipping seal for epoch " + epochNumber);
    return null;
  }

  const timestamp = Date.now();
  const payload = JSON.stringify({ epoch: epochNumber, timestamp, node_id: nodeId });
  const seal_hash = crypto.createHash("sha256").update(payload).digest("hex");
  const signature = signingFn ? signingFn(seal_hash) : null;

  const seal = { epoch_number: epochNumber, node_id: nodeId, timestamp, seal_hash, signature };
  receiveSeal(seal);
  return seal;
}

// ── Quorum resolution ─────────────────────────────────────────────────────────

function _resolveEpoch(epochNumber) {
  const state = epochState.get(epochNumber);
  if (!state || state.resolved) return;
  state.resolved = true;

  const seals = state.seals;

  if (seals.length === 0) {
    console.log("[BTCPC Clock] Epoch " + epochNumber + ": no seals received — skipping");
    emitter.emit("epoch_sealed", { epoch: epochNumber, sealed: false, quorum: 0 });
    return;
  }

  // Bootstrap: single clock, has external peers — self-sign allowed
  if (seals.length === 1) {
    if (externalPeerCount === 0 && seals[0].node_id !== "genesis") {
      console.warn("[BTCPC Clock] Epoch " + epochNumber +
        ": only self-seal and no external peers — deferring (potential isolation)");
      // Don't emit — let reconnection trigger re-seal
      return;
    }
    const winner = seals[0];
    state.winner = winner;
    _updateClockScore(winner.node_id, true);
    console.log("[BTCPC Clock] Epoch " + epochNumber + ": self-sealed (single clock, connected)");
    emitter.emit("epoch_sealed", { epoch: epochNumber, sealed: true, quorum: 1, winner, seals });
    _pruneOldEpochs(epochNumber);
    return;
  }

  // Outlier exclusion: compute median timestamp
  const timestamps = seals.map(s => s.timestamp).sort((a, b) => a - b);
  const median = timestamps[Math.floor(timestamps.length / 2)];
  const toleranceMs = OUTLIER_EPOCH_TOLERANCE * EPOCH_MS;

  const inliers = seals.filter(s => Math.abs(s.timestamp - median) <= toleranceMs);
  const outliers = seals.filter(s => Math.abs(s.timestamp - median) > toleranceMs);

  outliers.forEach(s => {
    _updateClockScore(s.node_id, false);
    console.warn("[BTCPC Clock] Epoch " + epochNumber + ": outlier clock " + s.node_id +
      " (deviation " + Math.round(Math.abs(s.timestamp - median) / 1000) + "s) — ignored");
  });

  // Check quorum: simple majority of inliers
  // Scale: as network grows, the inlier cluster naturally represents the honest majority
  const quorumNeeded = Math.max(1, Math.ceil(inliers.length * MIN_QUORUM_FRACTION));
  if (inliers.length < quorumNeeded) {
    console.warn("[BTCPC Clock] Epoch " + epochNumber +
      ": insufficient quorum (" + inliers.length + "/" + quorumNeeded + ") — deferring");
    // Could retry after more seals arrive; for now defer
    return;
  }

  // Winner: majority seal_hash among inliers
  const hashCount = new Map();
  for (const s of inliers) {
    hashCount.set(s.seal_hash, (hashCount.get(s.seal_hash) || 0) + 1);
  }
  const [winnerHash] = [...hashCount.entries()].sort((a, b) => b[1] - a[1])[0];
  const winnerSeals = inliers.filter(s => s.seal_hash === winnerHash);

  winnerSeals.forEach(s => _updateClockScore(s.node_id, true));
  inliers.filter(s => s.seal_hash !== winnerHash).forEach(s => _updateClockScore(s.node_id, false));

  const winner = winnerSeals[0];
  state.winner = winner;

  console.log("[BTCPC Clock] Epoch " + epochNumber + " sealed: quorum=" + winnerSeals.length +
    "/" + seals.length + " seals, outliers=" + outliers.length);

  emitter.emit("epoch_sealed", {
    epoch: epochNumber,
    sealed: true,
    quorum: winnerSeals.length,
    total_clocks: seals.length,
    outliers: outliers.length,
    winner,
    signing_clocks: winnerSeals.map(s => s.node_id),
    seals: winnerSeals,
  });

  _pruneOldEpochs(epochNumber);
}

// ── Clock scoring ─────────────────────────────────────────────────────────────

function _updateClockScore(nodeId, agreed) {
  const s = clockScores.get(nodeId) || { score: 100, outlier_count: 0, total_seals: 0 };
  s.total_seals++;
  s.last_seen = Date.now();
  if (agreed) {
    s.score = Math.min(200, s.score + 2);
  } else {
    s.outlier_count++;
    s.score = Math.max(0, s.score - 10);
  }
  clockScores.set(nodeId, s);
}

function getClockScores() {
  return Array.from(clockScores.entries()).map(([id, s]) => ({ node_id: id, ...s }));
}

// ── Helpers ───────────────────────────────────────────────────────────────────

let _getPeers = () => [];
function setPeerSource(fn) { _getPeers = fn; }

function getEpochState(epochNumber) {
  return epochState.get(epochNumber) || null;
}

function _pruneOldEpochs(currentEpoch) {
  for (const [epoch] of epochState) {
    if (epoch < currentEpoch - 20) epochState.delete(epoch);
  }
}

function on(event, fn) { emitter.on(event, fn); }

module.exports = {
  receiveSeal,
  produceSeal,
  updatePeerConnectivity,
  isObserver,
  getExternalPeerCount,
  getClockScores,
  getEpochState,
  setPeerSource,
  on,
  SEAL_COLLECT_MS,
  OUTLIER_EPOCH_TOLERANCE,
  ISOLATION_EPOCH_THRESHOLD,
};

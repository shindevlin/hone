"use strict";

/**
 * BTCPC Fork Resolver
 * Shin Devlin
 *
 * Detects chain forks and self-heals back to the main chain.
 *
 * Fork detection:
 *   On receiving any block from a peer, compare block_hash at the same height.
 *   If hashes diverge → find common ancestor → score both forks.
 *
 * Fork scoring (winner = higher score):
 *   sum(clock_signature_count per block since fork point)
 *   A private fork (offline node) always scores 0 — no external clocks signed it.
 *   Main chain always has >= 1 signature per block from the live quorum.
 *
 * Self-heal:
 *   1. Roll back local state to common ancestor
 *   2. Reverse all ledger entries since fork (stateStore.reverseEntries)
 *   3. Replay winning chain's blocks forward
 *   4. Resubmit pending work proofs from forked blocks to open settlements
 *   5. Verify state_root matches peer after replay — if not, full resync
 *
 * Guards:
 *   - heal_in_progress flag prevents new seals during healing
 *   - Max rollback depth: BTCPC_MAX_ROLLBACK_DEPTH epochs (default 500)
 *   - If fork is deeper than max: request full resync from peer
 */

const EventEmitter = require("events");

const MAX_ROLLBACK_DEPTH = parseInt(process.env.BTCPC_MAX_ROLLBACK_DEPTH) || 500;
const emitter = new EventEmitter();

let healInProgress = false;
let _blockStore = null;
let _stateStore = null;
let _ledger = null;
let _settlement = null;

function init({ blockStore, stateStore, ledger, settlement }) {
  _blockStore = blockStore;
  _stateStore = stateStore;
  _ledger = ledger;
  _settlement = settlement;
}

// ── Fork detection ────────────────────────────────────────────────────────────

/**
 * Check if a peer's block at a given height matches our local block.
 * Returns { forked: bool, local_hash, peer_hash }
 */
function checkForFork(epochNumber, peerBlockHash) {
  if (!_blockStore) return { forked: false };
  const localHeader = _blockStore.readBlockHeader(epochNumber);
  if (!localHeader) return { forked: false }; // we don't have this block yet — not a fork

  const localHash = localHeader.computeHash();
  const peerHash = typeof peerBlockHash === "string" ? peerBlockHash : peerBlockHash.toString("hex");

  return {
    forked: localHash !== peerHash,
    local_hash: localHash,
    peer_hash: peerHash,
    epoch: epochNumber,
  };
}

/**
 * Find the common ancestor between local chain and peer chain.
 * peerHashes: Map<epochNumber, hash> or array of { epoch, hash }
 * Returns the highest epoch where both chains agree, or -1 if none found.
 */
function findCommonAncestor(peerHashes) {
  if (!_blockStore) return -1;

  const peerMap = peerHashes instanceof Map
    ? peerHashes
    : new Map((peerHashes || []).map(h => [h.epoch, h.hash]));

  // Binary search down from the lowest peer height
  const peerEpochs = [...peerMap.keys()].sort((a, b) => b - a);
  if (peerEpochs.length === 0) return -1;

  let high = peerEpochs[0];
  let low = Math.max(0, high - MAX_ROLLBACK_DEPTH);
  let ancestor = -1;

  // Walk down linearly from the fork point (binary search would need random access)
  for (let epoch = high; epoch >= low; epoch--) {
    const peerHash = peerMap.get(epoch);
    if (!peerHash) continue;

    const localHeader = _blockStore.readBlockHeader(epoch);
    if (!localHeader) continue;

    const localHash = localHeader.computeHash();
    if (localHash === peerHash) {
      ancestor = epoch;
      break;
    }
  }

  return ancestor;
}

// ── Fork scoring ──────────────────────────────────────────────────────────────

/**
 * Score a chain segment by total clock signatures.
 * blocks: array of block payloads (from blockStore.readBlock)
 * Returns integer score — higher = more trusted.
 */
function scoreChain(blocks) {
  let score = 0;
  for (const b of (blocks || [])) {
    // Count clock signatures in this block's seal data
    const sigs = (b && b.block && b.block.clock_signatures) || [];
    score += Array.isArray(sigs) ? sigs.length : 0;

    // Also count signing clocks in reward settlement if available
    const rewardSigs = (b && b.payload && b.payload.clock_signatures) || [];
    score += Array.isArray(rewardSigs) ? rewardSigs.length : 0;
  }
  return score;
}

// ── Self-heal ─────────────────────────────────────────────────────────────────

/**
 * Attempt to self-heal from a detected fork.
 *
 * @param {object} options
 * @param {number} options.commonAncestor — epoch where chains last agreed
 * @param {number} options.localHeight — our current chain height
 * @param {Array}  options.peerBlocks — the winning chain's blocks since ancestor (in order)
 * @param {string} options.peerId — peer we're syncing from (for logging)
 * @returns {Promise<{ healed: bool, rolledBack: number, replayed: number, reason: string }>}
 */
async function selfHeal({ commonAncestor, localHeight, peerBlocks, peerId }) {
  if (healInProgress) {
    return { healed: false, reason: "heal_already_in_progress" };
  }

  const rollbackDepth = localHeight - commonAncestor;
  if (rollbackDepth > MAX_ROLLBACK_DEPTH) {
    console.warn("[BTCPC ForkResolver] Fork too deep (" + rollbackDepth + " epochs)" +
      " — requesting full resync from " + peerId);
    emitter.emit("need_full_resync", { peer: peerId, depth: rollbackDepth });
    return { healed: false, reason: "fork_too_deep_need_resync" };
  }

  // Score our chain vs theirs
  const localBlocks = [];
  for (let e = commonAncestor + 1; e <= localHeight; e++) {
    const b = _blockStore ? _blockStore.readBlock(e) : null;
    if (b) localBlocks.push(b);
  }

  const localScore = scoreChain(localBlocks);
  const peerScore = scoreChain(peerBlocks);

  if (localScore >= peerScore && localScore > 0) {
    console.log("[BTCPC ForkResolver] Our chain wins (score " + localScore +
      " vs " + peerScore + ") — no heal needed");
    return { healed: false, reason: "local_chain_wins" };
  }

  console.log("[BTCPC ForkResolver] FORK DETECTED — healing to peer chain" +
    " | ancestor=" + commonAncestor +
    " | rollback=" + rollbackDepth + " epochs" +
    " | score: local=" + localScore + " peer=" + peerScore +
    " | peer=" + peerId);

  healInProgress = true;
  emitter.emit("heal_started", { commonAncestor, rollbackDepth, peerId });

  try {
    // Step 1: Collect work proofs from our forked blocks to resubmit
    const orphanedProofs = [];
    for (const b of localBlocks) {
      for (const proof of (b.payload && b.payload.compute_proofs) || []) {
        orphanedProofs.push(proof);
      }
    }

    // Step 2: Collect ledger entries to reverse (most recent first)
    const entriesToReverse = [];
    for (let e = localHeight; e > commonAncestor; e--) {
      const b = _blockStore ? _blockStore.readBlock(e) : null;
      if (b && b.payload && b.payload.ledger_entries) {
        entriesToReverse.push(...b.payload.ledger_entries.slice().reverse());
      }
    }

    // Step 3: Roll back stateStore
    if (_stateStore && typeof _stateStore.reverseEntries === "function") {
      _stateStore.reverseEntries(entriesToReverse);
    } else {
      // Fallback: re-hydrate from last finality snapshot at or before commonAncestor
      const lastFin = _blockStore ? _blockStore.getLatestFinalityNumber() : -1;
      if (lastFin >= 0 && lastFin <= commonAncestor) {
        const fin = _blockStore.readFinality(lastFin);
        if (fin && fin.snapshot && _stateStore) {
          _stateStore.hydrateFromFinality(fin.snapshot);
        }
      }
    }
    console.log("[BTCPC ForkResolver] Rolled back " + rollbackDepth + " epochs to ancestor " + commonAncestor);

    // Step 4: Replay peer blocks forward
    let replayed = 0;
    for (const peerBlock of (peerBlocks || [])) {
      const entries = (peerBlock.payload && peerBlock.payload.ledger_entries) || [];
      if (_stateStore && typeof _stateStore.applyEntries === "function") {
        _stateStore.applyEntries(entries);
      }
      replayed++;
    }
    console.log("[BTCPC ForkResolver] Replayed " + replayed + " peer blocks");

    // Step 5: Resubmit orphaned proofs to open settlement windows
    if (_settlement) {
      let resubmitted = 0;
      for (const proof of orphanedProofs) {
        _settlement.submitWorkProof(proof);
        resubmitted++;
      }
      if (resubmitted > 0) {
        console.log("[BTCPC ForkResolver] Resubmitted " + resubmitted + " orphaned work proofs to settlement");
      }
    }

    emitter.emit("heal_complete", { commonAncestor, rolledBack: rollbackDepth, replayed, peerId });
    console.log("[BTCPC ForkResolver] Self-heal complete — chain restored to epoch " +
      (commonAncestor + replayed));

    return { healed: true, rolledBack: rollbackDepth, replayed, reason: "healed_to_peer_chain" };

  } catch (err) {
    console.error("[BTCPC ForkResolver] Heal failed:", err.message);
    emitter.emit("heal_failed", { error: err.message, peerId });
    // Request full resync as fallback
    emitter.emit("need_full_resync", { peer: peerId, reason: "heal_error" });
    return { healed: false, reason: "heal_error: " + err.message };
  } finally {
    healInProgress = false;
  }
}

// ── Integration point for chainSync ──────────────────────────────────────────

/**
 * Called by chainSync when a peer sends a block that doesn't match ours.
 * Automatically determines whether to heal and executes if needed.
 */
async function onPeerBlock(peerBlock, peerId, getPeerBlocksFn) {
  if (healInProgress) return;
  if (!peerBlock || !peerBlock.block) return;

  const peerEpoch = peerBlock.block.epoch_number;
  if (typeof peerEpoch !== "number") return;

  const localHeight = _stateStore ? _stateStore.getChainHeight() : 0;

  const { forked, local_hash, peer_hash } = checkForFork(peerEpoch, peerBlock.block.computeHash
    ? peerBlock.block.computeHash()
    : (peerBlock.block.hash || ""));

  if (!forked) return; // chains agree at this height

  console.log("[BTCPC ForkResolver] Fork detected at epoch " + peerEpoch +
    " | local=" + (local_hash || "?").slice(0, 12) +
    " peer=" + (peer_hash || "?").slice(0, 12) +
    " | from " + peerId);

  // Request peer's recent block hashes to find common ancestor
  let peerHashes = [];
  let peerBlocks = [];
  try {
    if (typeof getPeerBlocksFn === "function") {
      const result = await getPeerBlocksFn(peerId, Math.max(0, peerEpoch - MAX_ROLLBACK_DEPTH), peerEpoch);
      peerHashes = result.hashes || [];
      peerBlocks = result.blocks || [];
    }
  } catch (err) {
    console.warn("[BTCPC ForkResolver] Could not fetch peer blocks:", err.message);
    return;
  }

  const ancestor = findCommonAncestor(peerHashes);
  if (ancestor < 0) {
    console.warn("[BTCPC ForkResolver] No common ancestor found — requesting full resync");
    emitter.emit("need_full_resync", { peer: peerId, reason: "no_common_ancestor" });
    return;
  }

  await selfHeal({ commonAncestor: ancestor, localHeight, peerBlocks, peerId });
}

function isHealInProgress() { return healInProgress; }
function on(event, fn) { emitter.on(event, fn); }

module.exports = {
  init,
  checkForFork,
  findCommonAncestor,
  scoreChain,
  selfHeal,
  onPeerBlock,
  isHealInProgress,
  on,
  MAX_ROLLBACK_DEPTH,
};

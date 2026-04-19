"use strict";

/**
 * BTCPC Inference Protocol
 * Shin Devlin
 *
 * P2P inference request/claim/commit/reveal protocol.
 * Requests flow through the network, not direct HTTP.
 *
 * Message types:
 *   INFERENCE_REQUEST  — user broadcasts: "I need this computed"
 *   INFERENCE_CLAIM    — node responds: "I'll do it" (includes SIK hash)
 *   INFERENCE_ASSIGN   — network assigns N nodes to the request
 *   INFERENCE_PAYLOAD  — user sends SIK-encrypted prompt to each assigned node
 *   INFERENCE_COMMIT   — node commits result hash (before revealing)
 *   INFERENCE_REVEAL   — node reveals encrypted result (after all commits in)
 *   INFERENCE_RESULT   — final consensus result delivered to user
 */

const crypto = require("crypto");

// ── Services ──
const stateStore = require("../chain/stateStore");
const ledger = require("../services/ledger");
const escrow = require("../services/escrow");

// ── Sharding modules (lazy-required to avoid circular deps) ──
// Loaded on first use only
let _ensembleCoordinator = null;
function getEnsembleCoordinator() {
  if (!_ensembleCoordinator) _ensembleCoordinator = require('./ensembleCoordinator');
  return _ensembleCoordinator;
}

// ── Constants ──
const CLAIM_WINDOW_MS = 15000;      // 15s for nodes to claim
const COMMIT_TIMEOUT_MS = 120000;   // 2min for nodes to commit results
const REVEAL_TIMEOUT_MS = 30000;    // 30s for reveal after all commits
const MIN_REDUNDANCY = 1;           // N=1 for solo/early network
const STANDARD_REDUNDANCY = 3;      // N=3 for mature network

// ── Active requests in memory ──
const activeRequests = new Map();

/**
 * Phase 1: User creates an inference request.
 * Returns a request object to broadcast via P2P.
 */
async function createInferenceRequest({
  requesterId,
  requesterPublicKey,
  model,
  promptHash,
  maxFee,
  redundancy,
  mode,
  ensembleMinSize,
  ensembleDeadlineMs,
}) {
  const requestId = crypto.randomBytes(16).toString("hex");
  const fee = maxFee || 10;

  // Normalize mode: default to 'solo' if not specified
  const reqMode = mode === 'ensemble' ? 'ensemble'
    : mode === 'pipeline' ? 'pipeline'
    : 'solo';

  // Lock funds in escrow
  await escrow.lockFunds(requestId, requesterId, fee);

  const request = {
    type: "INFERENCE_REQUEST",
    request_id: requestId,
    requester_id: requesterId,
    requester_public_key: requesterPublicKey,
    model: model || "qwen3.5:27b",
    prompt_hash: promptHash,
    max_fee: fee,
    redundancy: redundancy || MIN_REDUNDANCY,
    mode: reqMode,
    timestamp: Date.now(),
    status: reqMode === 'solo' ? "open" : "broadcasting",
    claims: [],
    assignments: [],
    commits: [],
    reveals: [],
  };

  activeRequests.set(requestId, request);

  // ── Mode A: Ensemble ──
  // Skip claim/assign dance; register a coordinator job so contributions
  // can be tracked. The caller must broadcast INFERENCE_PAYLOAD to all peers.
  if (reqMode === 'ensemble') {
    const minSize = ensembleMinSize || 3;
    const deadline = ensembleDeadlineMs || (Date.now() + COMMIT_TIMEOUT_MS);
    getEnsembleCoordinator().createEnsembleJob(
      requestId,
      model || "qwen3.5:27b",
      promptHash,
      minSize,
      deadline
    );
    console.log(`[BTCPC Inference] Ensemble job created: ${requestId.slice(0, 12)} minSize=${minSize}`);
  }

  // ── Mode B: Pipeline ──
  // The shard group coordinator handles routing; recorded here for tracking.
  if (reqMode === 'pipeline') {
    console.log(`[BTCPC Inference] Pipeline request created: ${requestId.slice(0, 12)} model=${model}`);
  }

  return request;
}

/**
 * Phase 2: Node claims an inference request.
 * Node must have sufficient stake and an active SIK.
 */
async function claimRequest(requestId, nodeId, sikHash, price) {
  const request = activeRequests.get(requestId);
  if (!request) return { error: "Request not found" };
  if (request.status !== "open") return { error: "Request no longer open" };

  // Verify node is registered (no Mongo)
  const nodeRegistry = require('../chain/nodeRegistry');
  const nodeEntry = nodeRegistry.getNode(nodeId);
  if (!nodeEntry) return { error: 'Node not registered' };

  // Check if node is already busy with active inference
  const nodeStr = nodeId.toString();
  let activeClaims = 0;
  for (const [, req] of activeRequests) {
    if (req.status === "assigned" || req.status === "committing") {
      if (req.assignments.some((a) => a.node_id === nodeStr)) {
        activeClaims++;
      }
    }
  }
  if (activeClaims >= 1) {
    return { error: "Node at capacity", active: activeClaims, max: 1 };
  }

  // Check price within user's max fee
  if (price > request.max_fee) return { error: "Price exceeds max fee" };

  const claim = {
    type: "INFERENCE_CLAIM",
    request_id: requestId,
    node_id: nodeId.toString(),
    sik_hash: sikHash,
    price,
    timestamp: Date.now(),
  };

  request.claims.push(claim);
  return claim;
}

/**
 * Phase 3: Assign nodes after claim window closes.
 * Selects N nodes by lowest price, highest reputation.
 * Returns assignments with SIK hashes so user can encrypt per-node.
 */
async function assignNodes(requestId) {
  const request = activeRequests.get(requestId);
  if (!request) return { error: "Request not found" };
  if (request.claims.length === 0) return { error: "No claims received" };

  const N = Math.min(request.redundancy, request.claims.length);

  // ── Anti-centralization assignment ──
  // Prevents powerful nodes from dominating all work.
  //
  // Score = base_score - concentration_penalty + newcomer_bonus
  //
  // - base_score: lower price = better
  // - concentration_penalty: nodes that did more work recently get penalized
  // - newcomer_bonus: nodes with < 100 epochs get a boost
  //
  // This ensures new miners get a fair shot at work even against
  // established nodes with better hardware.

  const nodeRegistry = require('../chain/nodeRegistry');
  const scoredClaims = await Promise.all(
    request.claims.map(async (claim) => {
      const nodeEntry = nodeRegistry.getNode(claim.node_id);
      const reputation = 100; // default; real reputation tracked in stateStore later
      const epochsDone = nodeEntry ? (nodeEntry.registeredEpoch || 0) : 0;

      // Base: lower price = higher score (invert, normalize)
      const priceScore = 1.0 - claim.price / (request.max_fee || 10);

      // Concentration penalty: more work done = less priority for new jobs
      const concentrationPenalty = Math.min(0.4, Math.log10(epochsDone + 1) * 0.1);

      // Newcomer bonus: nodes with < 100 epochs get up to 0.3 boost
      const newcomerBonus = epochsDone < 100 ? 0.3 * (1 - epochsDone / 100) : 0;

      // Reputation factor (0.5 to 1.0 range)
      const repFactor = 0.5 + (reputation / 200);

      // Stake bonus: logarithmic, capped at +0.2
      const stakeAmt = nodeEntry ? (nodeEntry.stake || 1000) : 1000;
      const stakeBonus = Math.min(0.2, 0.2 * Math.log(stakeAmt / 1000) / Math.log(100));

      const score =
        priceScore * repFactor - concentrationPenalty + newcomerBonus + Math.max(0, stakeBonus);

      return { ...claim, score, epochsDone, newcomerBonus, stakeBonus };
    })
  );

  // Sort by score descending
  scoredClaims.sort((a, b) => b.score - a.score);

  const assigned = scoredClaims.slice(0, N);
  request.assignments = assigned.map((c) => ({
    node_id: c.node_id,
    sik_hash: c.sik_hash,
    price: c.price,
  }));
  request.status = "assigned";

  return {
    type: "INFERENCE_ASSIGN",
    request_id: requestId,
    assignments: request.assignments,
  };
}

/**
 * Phase 4: User sends encrypted payload to each assigned node.
 * Each payload is encrypted with that node's SIK-bound session key.
 * The user opens a session with each node and encrypts separately.
 */
function receivePayload(requestId, nodeId, encryptedPayload, sessionId) {
  const request = activeRequests.get(requestId);
  if (!request) return { error: "Request not found" };

  const assignment = request.assignments.find(
    (a) => a.node_id === nodeId.toString()
  );
  if (!assignment) return { error: "Node not assigned to this request" };

  assignment.payload = encryptedPayload;
  assignment.session_id = sessionId;
  assignment.payload_received = Date.now();

  return {
    type: "INFERENCE_PAYLOAD",
    request_id: requestId,
    node_id: nodeId.toString(),
    received: true,
  };
}

/**
 * Phase 5: Node commits result hash (before revealing actual result).
 * Commit-reveal prevents result copying between nodes.
 */
function commitResult(requestId, nodeId, resultHash) {
  const request = activeRequests.get(requestId);
  if (!request) return { error: "Request not found" };
  if (request.status !== "assigned" && request.status !== "committing") {
    return { error: "Request not in commit phase" };
  }

  const assignment = request.assignments.find(
    (a) => a.node_id === nodeId.toString()
  );
  if (!assignment) return { error: "Node not assigned" };

  const commit = {
    type: "INFERENCE_COMMIT",
    request_id: requestId,
    node_id: nodeId.toString(),
    result_hash: resultHash,
    timestamp: Date.now(),
  };

  request.commits.push(commit);
  request.status = "committing";

  // Check if all assigned nodes have committed
  if (request.commits.length >= request.assignments.length) {
    request.status = "revealing";
  }

  return commit;
}

/**
 * Phase 6: Node reveals encrypted result after all commits are in.
 * Network compares result hashes to determine consensus.
 */
async function revealResult(requestId, nodeId, encryptedResult, resultHash) {
  const request = activeRequests.get(requestId);
  if (!request) return { error: "Request not found" };
  if (request.status !== "revealing") return { error: "Not in reveal phase" };

  // Verify hash matches commit
  const commit = request.commits.find(
    (c) => c.node_id === nodeId.toString()
  );
  if (!commit) return { error: "No commit from this node" };
  if (commit.result_hash !== resultHash) {
    return { error: "Result hash does not match commit", slashable: true };
  }

  const reveal = {
    type: "INFERENCE_REVEAL",
    request_id: requestId,
    node_id: nodeId.toString(),
    encrypted_result: encryptedResult,
    result_hash: resultHash,
    timestamp: Date.now(),
  };

  request.reveals.push(reveal);

  // Check consensus when all reveals are in
  if (request.reveals.length >= request.assignments.length) {
    return await finalizeRequest(requestId);
  }

  return reveal;
}

/**
 * Phase 7: Determine consensus and deliver result.
 * Matching hashes = correct answer. Non-matching = slashed.
 *
 * For ensemble mode, this is called when the ensemble coordinator has declared
 * consensus (or after deadline). `ensembleOutcome` carries the pre-computed
 * consensus result from ensembleCoordinator.submitContribution().
 */
async function finalizeRequest(requestId, ensembleOutcome) {
  const request = activeRequests.get(requestId);
  if (!request) return { error: "Request not found" };

  // ── Ensemble finalization path ──
  if (request.mode === 'ensemble' && ensembleOutcome) {
    const ec = getEnsembleCoordinator();
    const job = ec.getJob(requestId);
    const consensusNodes = ensembleOutcome.consensusNodes || [];
    const totalFee = request.max_fee;

    // Equal split among consensus nodes
    const perNodeShare = consensusNodes.length > 0
      ? totalFee / consensusNodes.length
      : 0;

    const payouts = consensusNodes.map((nodeId) => ({
      node_id: nodeId,
      amount: perNodeShare,
      rank: 1,
    }));

    request.status = "finalized";
    request.consensus_hash = ensembleOutcome.result ? ensembleOutcome.result.result_hash : null;
    request.honest_nodes = consensusNodes;
    request.dishonest_nodes = [];
    request.payouts = payouts;

    const result = {
      type: "INFERENCE_RESULT",
      request_id: requestId,
      mode: "ensemble",
      consensus_hash: request.consensus_hash,
      result_text: ensembleOutcome.result ? ensembleOutcome.result.result_text : null,
      honest_nodes: consensusNodes,
      dishonest_nodes: [],
      payouts,
      timestamp: Date.now(),
    };

    try {
      await escrow.releaseFunds(requestId, payouts);
    } catch (err) {
      console.error("[BTCPC Escrow] Ensemble release error:", err.message);
    }

    setImmediate(async () => {
      try {
        const currentEpoch = 0;
        for (const payout of payouts) {
          await ledger.recordInferenceCharge(
            request.requester_id,
            payout.node_id,
            payout.amount,
            currentEpoch,
            requestId
          );
        }
      } catch (err) {
        console.error('[BTCPC Inference] Ensemble charge record error:', err.message);
      }
    });

    persistRequest(request).catch((err) =>
      console.error("[BTCPC Inference] Persist error:", err.message)
    );

    return result;
  }

  // Count matching hashes
  const hashCounts = {};
  for (const reveal of request.reveals) {
    hashCounts[reveal.result_hash] = hashCounts[reveal.result_hash] || [];
    hashCounts[reveal.result_hash].push(reveal);
  }

  // Find the majority hash
  let consensusHash = null;
  let consensusReveals = [];
  for (const [hash, reveals] of Object.entries(hashCounts)) {
    if (reveals.length > consensusReveals.length) {
      consensusHash = hash;
      consensusReveals = reveals;
    }
  }

  // Identify honest and dishonest nodes
  const honest = consensusReveals.map((r) => r.node_id);
  const dishonest = request.reveals
    .filter((r) => r.result_hash !== consensusHash)
    .map((r) => r.node_id);

  // Payment distribution (from whitepaper):
  //   First matching: 50%, Second: 30%, Third: 20%
  const totalFee = request.assignments[0]?.price || request.max_fee;
  const payouts = [];
  const shares = [0.5, 0.3, 0.2];
  for (let i = 0; i < honest.length && i < shares.length; i++) {
    payouts.push({
      node_id: honest[i],
      amount: totalFee * shares[i],
      rank: i + 1,
    });
  }

  // Winner's encrypted result goes to the user
  const winningReveal = consensusReveals[0];

  request.status = "finalized";
  request.consensus_hash = consensusHash;
  request.honest_nodes = honest;
  request.dishonest_nodes = dishonest;
  request.payouts = payouts;

  // ── Slash dishonest nodes via slashing service ──
  // (async slashing happens in background — finalizeRequest itself is sync)
  if (dishonest.length > 0) {
    setImmediate(async () => {
      const slashing = require('../services/slashing');
      const currentEpoch = request.assignments[0]?.epoch || 0;
      for (const nodeId of dishonest) {
        try {
          await slashing.recordOffense(nodeId, 'COLLUSION', {
            request_id: requestId,
            epoch: currentEpoch,
            result_hash: request.consensus_hash
          });
        } catch (err) {
          console.error(`[BTCPC Slash] Error slashing node ${nodeId}:`, err.message);
        }
      }
    });
  }

  const result = {
    type: "INFERENCE_RESULT",
    request_id: requestId,
    consensus_hash: consensusHash,
    encrypted_result: winningReveal.encrypted_result,
    honest_nodes: honest,
    dishonest_nodes: dishonest,
    payouts,
    timestamp: Date.now(),
  };

  // Release escrow to honest nodes
  try {
    await escrow.releaseFunds(requestId, payouts);
  } catch (err) {
    console.error("[BTCPC Escrow] Release error:", err.message);
  }

  // Record inference charge on-chain — deducts from requester's delegated balance first
  setImmediate(async () => {
    try {
      const ledger = require('../services/ledger');
      const currentEpoch = request.assignments[0]?.epoch || 0;
      const requester = request.requester_account || request.requester_id;
      if (requester && totalFee > 0) {
        for (const payout of payouts) {
          await ledger.recordInferenceCharge(requester, payout.node_id, payout.amount, currentEpoch, requestId);
        }
      }
    } catch (err) {
      console.error('[BTCPC Inference] Charge record error:', err.message);
    }
  });

  // Persist to DB
  persistRequest(request).catch((err) =>
    console.error("[BTCPC Inference] Persist error:", err.message)
  );

  return result;
}

/**
 * Clean up a finalized request from the active map.
 * The permanent record lives in the block ledger via escrow/reward entries.
 */
async function persistRequest(request) {
  // Clean up from active map
  activeRequests.delete(request.request_id);
}

/**
 * Get request status.
 */
function getRequest(requestId) {
  return activeRequests.get(requestId) || null;
}

/**
 * Get all active requests (for nodes to browse and claim).
 */
function getOpenRequests() {
  const open = [];
  for (const [id, req] of activeRequests) {
    if (req.status === "open") {
      open.push({
        request_id: id,
        model: req.model,
        max_fee: req.max_fee,
        redundancy: req.redundancy,
        claims: req.claims.length,
        timestamp: req.timestamp,
      });
    }
  }
  return open;
}

module.exports = {
  createInferenceRequest,
  claimRequest,
  assignNodes,
  receivePayload,
  commitResult,
  revealResult,
  getRequest,
  getOpenRequests,
  activeRequests,
};

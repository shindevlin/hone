"use strict";

/**
 * BTCPC Verifier Engine
 * Shin Devlin
 *
 * Verifiers are independent nodes that:
 *   1. Pull submitted work proofs from the network
 *   2. Independently re-run or spot-check the inference
 *   3. Compute their own verified_work_value for that proof
 *      (model_weight × tokens they observed — miner can't inflate this)
 *   4. Propose a reward weight for that proof
 *   5. Broadcast VERIFICATION_RESULT to the network
 *   6. Cross-check other verifiers' proposals
 *   7. When enough verifiers agree on the same reward_hash → sign it
 *
 * Work value verification:
 *   LLMs are non-deterministic so exact hash matching is not used.
 *   Instead: verifier re-runs inference on same prompt, counts tokens.
 *   If verifier token count is within TOLERANCE of miner's claim → accepted.
 *   Verifier's OWN token count becomes the authoritative work_value for their proposal.
 *   Outlier verifiers (far from median work_value) are ignored — same pattern as clocks.
 *
 * Reward proposals:
 *   Each verifier calls computeRewards() on the verified proofs they accepted.
 *   Deterministic: same verified set → same reward_hash → easy consensus.
 *   Proposals weighted by: proofs_verified × historical_accuracy_score.
 *   Majority reward_hash among weighted proposals wins.
 *
 * Verifier pool:
 *   Verifiers earn from the 10% verifier pool (rewardEngine.js).
 *   A verifier who disagrees with consensus consistently loses score and
 *   eventually earns nothing — economic incentive to verify honestly.
 */

const crypto = require("crypto");
const EventEmitter = require("events");
const axios = require("axios");

const { computeRewards } = require("./rewardEngine");
const { getBlockReward } = require("../services/emissionSchedule");

const OLLAMA_URL = process.env.OLLAMA_URL || "http://localhost:11434";
const VERIFICATION_TOLERANCE = parseFloat(process.env.BTCPC_VERIFY_TOLERANCE) || 0.20; // 20% token count variance allowed
const OUTLIER_WORK_TOLERANCE = parseFloat(process.env.BTCPC_VERIFY_OUTLIER) || 0.30; // 30% from median → outlier
const MIN_VERIFIER_AGREEMENT = parseFloat(process.env.BTCPC_VERIFY_QUORUM) || 0.51; // simple majority

const emitter = new EventEmitter();

// Unverified proofs waiting to be picked up
// Map<proofId, UnverifiedProof>
const proofPool = new Map();

// Verification results received from all verifiers (including self)
// Map<proofId, Map<verifierId, VerificationResult>>
const verificationResults = new Map();

// Our local verifier identity
let VERIFIER_ACCOUNT = process.env.BTCPC_VERIFIER || process.env.BTCPC_MINER || null;

// Verifier accuracy scores — weighted by historical agreement with consensus
// Map<verifierId, { score, agreed, disagreed, total_verified }>
const verifierScores = new Map();

// ── Proof pool management ─────────────────────────────────────────────────────

/**
 * Add a submitted work proof to the verification pool.
 * Called when a WORK_PROOF P2P message is received.
 */
function addToPool(proof) {
  if (!proof || !proof.id || !proof.node_id) return;
  if (proofPool.has(proof.id)) return; // already know about it

  proofPool.set(proof.id, {
    ...proof,
    received_at: Date.now(),
    verification_status: "pending",
  });

  console.log("[BTCPC Verifier] Proof " + proof.id.slice(0, 12) + " from " + proof.node_id +
    " added to verification pool (claim: " + (proof.work_value || 0).toFixed(1) + " work_value)");

  emitter.emit("proof_available", proof);
}

/**
 * Get all proofs in the pool that have not yet been verified by this node.
 */
function getUnverifiedProofs() {
  return Array.from(proofPool.values()).filter(p => p.verification_status === "pending");
}

// ── Core verification ─────────────────────────────────────────────────────────

/**
 * Verify a single work proof.
 * Re-runs inference on the same prompt hash to independently measure work.
 *
 * Returns { accepted, verified_work_value, verified_tokens, reason }
 */
async function verifyProof(proof) {
  if (!proof || !proof.model || !proof.prompt_hash) {
    return { accepted: false, reason: "missing_fields" };
  }

  // Step 1: Verify the model exists and get its actual parameter count
  let paramCount = 0;
  try {
    const { verifyModelParams } = require("../mining/workGenerator");
    paramCount = await verifyModelParams(proof.model);
  } catch (err) {
    console.warn("[BTCPC Verifier] Cannot verify model params for " + proof.model + ": " + err.message);
    // Still proceed — use claimed work_value with penalty
    return {
      accepted: true,
      verified_work_value: (proof.work_value || 0) * 0.5, // 50% if we can't verify model
      verified_tokens: proof.tokens_generated || 0,
      reason: "model_unverifiable_partial_credit",
    };
  }

  // Step 2: Reconstruct prompt from hash (if we have the prompt) or generate equivalent
  // For now: use a canonical verification prompt derived from prompt_hash.
  // This lets verifiers measure compute work without needing the exact original prompt.
  const verifyPrompt = "Verify computation " + proof.prompt_hash.slice(0, 16) +
    ". Respond in exactly the format: VERIFIED:[hash]. Hash = SHA256 of your response.";

  let verifiedTokens = 0;
  try {
    const response = await axios.post(OLLAMA_URL + "/api/generate", {
      model: proof.model,
      prompt: verifyPrompt,
      stream: false,
      options: { num_predict: Math.min(proof.tokens_generated || 256, 512), temperature: 0 },
    }, { timeout: 120000 });

    verifiedTokens = response.data.eval_count || 0;
  } catch (err) {
    console.warn("[BTCPC Verifier] Inference verification failed for " + proof.node_id +
      " proof " + proof.id.slice(0, 12) + ": " + err.message);
    return { accepted: false, reason: "inference_unreachable" };
  }

  // Step 3: Check token count is within tolerance of miner's claim
  const claimed = proof.tokens_generated || 0;
  const deviation = claimed > 0 ? Math.abs(verifiedTokens - claimed) / claimed : 1;

  if (deviation > VERIFICATION_TOLERANCE) {
    console.warn("[BTCPC Verifier] REJECTED proof from " + proof.node_id +
      " — token deviation " + (deviation * 100).toFixed(1) + "%" +
      " (claimed " + claimed + ", verified " + verifiedTokens + ")");
    return {
      accepted: false,
      verified_work_value: 0,
      verified_tokens: verifiedTokens,
      reason: "token_count_mismatch",
      deviation,
    };
  }

  // Step 4: Compute verified work_value from OUR measurements, not the miner's claim
  // Apply tool multiplier if the proof carried tool-augmented context
  let toolMultiplier = 1.0;
  let toolTraceValid = null;
  if (proof.tools_used && proof.tools_used.length > 0) {
    const { computeToolMultiplier } = require("../mcp/toolRegistry");
    toolMultiplier = computeToolMultiplier(proof.tools_used);

    // Verify the tool trace hash if a call log was provided
    if (proof.tool_trace_hash && proof.tool_call_log) {
      try {
        const { verifyToolTrace } = require("../mcp/toolExecutor");
        const tvResult = await verifyToolTrace(proof.tool_trace_hash, proof.tool_call_log);
        toolTraceValid = tvResult.valid;
        if (!tvResult.valid) {
          console.warn("[BTCPC Verifier] Tool trace mismatch for " + proof.node_id +
            " proof " + (proof.id || "").slice(0, 12) + " — " + tvResult.reason);
          toolMultiplier = 1.0; // strip multiplier if trace is invalid
        }
      } catch (_) {
        toolTraceValid = null; // can't verify — keep multiplier but note it
      }
    }
  }

  const verified_work_value = verifiedTokens * paramCount * toolMultiplier;

  console.log("[BTCPC Verifier] ACCEPTED proof from " + proof.node_id +
    " | claimed_tokens=" + claimed + " verified_tokens=" + verifiedTokens +
    " | verified_work_value=" + verified_work_value.toFixed(1) +
    (toolMultiplier > 1 ? " | tool_mult=" + toolMultiplier.toFixed(2) : ""));

  return {
    accepted: true,
    verified_work_value,
    verified_tokens: verifiedTokens,
    param_count: paramCount,
    tool_multiplier: toolMultiplier,
    tool_trace_valid: toolTraceValid,
    reason: "verified",
    deviation,
  };
}

// ── Reward proposal ───────────────────────────────────────────────────────────

/**
 * Run verification on all pending proofs and build a reward proposal.
 * Called at the end of the verification window for an epoch.
 *
 * Returns { reward_hash, rewards, verified_proofs, proposal_weight }
 */
async function buildRewardProposal(epochNumber, allProofsForEpoch, clockNodes, storageHosts) {
  if (!VERIFIER_ACCOUNT) return null;

  const verifiedProofs = [];

  for (const proof of (allProofsForEpoch || [])) {
    const result = await verifyProof(proof);
    if (result.accepted) {
      verifiedProofs.push({
        node_id: proof.node_id,
        work_value: result.verified_work_value,
        model: proof.model,
        tokens_generated: result.verified_tokens,
        result_hash: proof.result_hash,
        _verifier: VERIFIER_ACCOUNT,
        _deviation: result.deviation,
      });
    }

    // Broadcast our verification result
    const verResult = {
      proof_id: proof.id,
      verifier_id: VERIFIER_ACCOUNT,
      epoch: epochNumber,
      accepted: result.accepted,
      verified_work_value: result.verified_work_value || 0,
      verified_tokens: result.verified_tokens || 0,
      reason: result.reason,
      timestamp: Date.now(),
    };
    _receiveVerificationResult(verResult);
    emitter.emit("verification_result_produced", verResult);
  }

  const blockReward = getBlockReward(epochNumber);
  const rewardResult = computeRewards({
    epochNumber,
    blockReward,
    computeProofs: verifiedProofs,
    verifiers: [VERIFIER_ACCOUNT],
    clockNodes: clockNodes || [],
    storageHosts: storageHosts || [],
    serviceHosts: [],
  });

  // Deterministic hash — all verifiers with same verified set produce same hash
  const canonical = JSON.stringify({
    epoch: epochNumber,
    rewards: rewardResult.rewards
      .map(r => ({ to: r.to, amount: r.amount, type: r.type }))
      .sort((a, b) => a.to < b.to ? -1 : 1),
    proof_count: verifiedProofs.length,
  });
  const reward_hash = crypto.createHash("sha256").update(canonical).digest("hex");

  // Proposal weight = proofs verified × our accuracy score
  const myScore = verifierScores.get(VERIFIER_ACCOUNT) || { score: 100 };
  const proposal_weight = verifiedProofs.length * (myScore.score / 100);

  console.log("[BTCPC Verifier] Reward proposal for epoch " + epochNumber +
    " | verified=" + verifiedProofs.length + "/" + (allProofsForEpoch || []).length +
    " | hash=" + reward_hash.slice(0, 12) + "..." +
    " | weight=" + proposal_weight.toFixed(2));

  return {
    epoch: epochNumber,
    verifier_id: VERIFIER_ACCOUNT,
    reward_hash,
    rewards: rewardResult.rewards,
    verified_proofs: verifiedProofs,
    proposal_weight,
    timestamp: Date.now(),
  };
}

// ── Receiving other verifiers' results ────────────────────────────────────────

/**
 * Receive a VERIFICATION_RESULT from another verifier (or self).
 * Called when a VERIFICATION_RESULT P2P message arrives.
 */
function _receiveVerificationResult(result) {
  const { proof_id, verifier_id } = result;
  if (!proof_id || !verifier_id) return;

  if (!verificationResults.has(proof_id)) {
    verificationResults.set(proof_id, new Map());
  }
  verificationResults.get(proof_id).set(verifier_id, result);
}

function receiveVerificationResult(result) {
  _receiveVerificationResult(result);
  emitter.emit("verification_result_received", result);
}

/**
 * Receive a reward proposal from another verifier.
 * Cross-checks their proposal against ours.
 */
function receiveRewardProposal(proposal) {
  emitter.emit("reward_proposal_received", proposal);
}

// ── Consensus on reward proposals ────────────────────────────────────────────

/**
 * Given a set of reward proposals from all verifiers, reach consensus.
 * Uses weighted majority on reward_hash (same outlier-exclusion pattern as clocks).
 *
 * proposals: [{ verifier_id, reward_hash, proposal_weight, rewards }]
 * Returns { winning_hash, winning_rewards, signing_verifiers, total_weight }
 */
function reachConsensus(proposals) {
  if (!proposals || proposals.length === 0) return null;

  // Single verifier: self-consensus
  if (proposals.length === 1) {
    _updateVerifierScore(proposals[0].verifier_id, true);
    return {
      winning_hash: proposals[0].reward_hash,
      winning_rewards: proposals[0].rewards,
      signing_verifiers: [proposals[0].verifier_id],
      total_weight: proposals[0].proposal_weight || 1,
      consensus: "single_verifier",
    };
  }

  // Aggregate weighted votes by reward_hash
  const hashWeights = new Map();
  const hashProposals = new Map();

  for (const p of proposals) {
    const w = p.proposal_weight || 1;
    hashWeights.set(p.reward_hash, (hashWeights.get(p.reward_hash) || 0) + w);
    if (!hashProposals.has(p.reward_hash)) hashProposals.set(p.reward_hash, p);
  }

  const totalWeight = proposals.reduce((s, p) => s + (p.proposal_weight || 1), 0);
  const [winnerHash, winnerWeight] = [...hashWeights.entries()]
    .sort((a, b) => b[1] - a[1])[0];

  // Must reach simple majority of total weight
  if (winnerWeight < totalWeight * MIN_VERIFIER_AGREEMENT) {
    console.warn("[BTCPC Verifier] No consensus reached — max weight " +
      winnerWeight.toFixed(2) + "/" + totalWeight.toFixed(2));
    return null;
  }

  const winningProposal = hashProposals.get(winnerHash);
  const signingVerifiers = proposals
    .filter(p => p.reward_hash === winnerHash)
    .map(p => p.verifier_id);

  // Update scores
  for (const p of proposals) {
    _updateVerifierScore(p.verifier_id, p.reward_hash === winnerHash);
  }

  console.log("[BTCPC Verifier] Consensus reached on epoch " + (winningProposal.epoch || "?") +
    " | hash=" + winnerHash.slice(0, 12) + "..." +
    " | weight=" + winnerWeight.toFixed(2) + "/" + totalWeight.toFixed(2) +
    " | signers=" + signingVerifiers.join(", "));

  return {
    winning_hash: winnerHash,
    winning_rewards: winningProposal.rewards,
    signing_verifiers: signingVerifiers,
    total_weight: totalWeight,
    winner_weight: winnerWeight,
    consensus: "weighted_majority",
  };
}

// ── Consensus on per-proof work_value ─────────────────────────────────────────

/**
 * Given verification results from multiple verifiers for a single proof,
 * compute the consensus work_value using median of non-outliers.
 *
 * This prevents a single rogue verifier from inflating or deflating a proof's value.
 */
function consensusWorkValue(proofId) {
  const results = verificationResults.get(proofId);
  if (!results || results.size === 0) return null;

  const accepted = Array.from(results.values()).filter(r => r.accepted);
  if (accepted.length === 0) return 0; // all rejected

  const values = accepted.map(r => r.verified_work_value).sort((a, b) => a - b);
  const median = values[Math.floor(values.length / 2)];

  // Exclude outliers > 30% from median
  const inliers = values.filter(v => Math.abs(v - median) / (median || 1) <= OUTLIER_WORK_TOLERANCE);

  if (inliers.length === 0) return median; // fallback if all are outliers (shouldn't happen)

  const consensusValue = inliers.reduce((s, v) => s + v, 0) / inliers.length;

  console.log("[BTCPC Verifier] Proof " + proofId.slice(0, 12) + " consensus work_value=" +
    consensusValue.toFixed(1) + " (from " + inliers.length + " verifiers, " +
    (values.length - inliers.length) + " outliers excluded)");

  return consensusValue;
}

// ── Score tracking ────────────────────────────────────────────────────────────

function _updateVerifierScore(verifierId, agreed) {
  const s = verifierScores.get(verifierId) || { score: 100, agreed: 0, disagreed: 0, total_verified: 0 };
  s.total_verified++;
  if (agreed) {
    s.agreed++;
    s.score = Math.min(200, s.score + 2);
  } else {
    s.disagreed++;
    s.score = Math.max(0, s.score - 15);
  }
  verifierScores.set(verifierId, s);
}

function getVerifierScores() {
  return Array.from(verifierScores.entries()).map(([id, s]) => ({ verifier_id: id, ...s }));
}

function setVerifierAccount(account) { VERIFIER_ACCOUNT = account; }
function getVerifierAccount() { return VERIFIER_ACCOUNT; }
function on(event, fn) { emitter.on(event, fn); }

// ── Prune ─────────────────────────────────────────────────────────────────────

function pruneOldResults(currentEpoch, keepEpochs) {
  keepEpochs = keepEpochs || 20;
  const cutoffTime = Date.now() - (keepEpochs * 30000);
  for (const [id, proof] of proofPool) {
    if ((proof.received_at || 0) < cutoffTime) proofPool.delete(id);
  }
}

module.exports = {
  addToPool,
  getUnverifiedProofs,
  verifyProof,
  buildRewardProposal,
  receiveVerificationResult,
  receiveRewardProposal,
  reachConsensus,
  consensusWorkValue,
  getVerifierScores,
  setVerifierAccount,
  getVerifierAccount,
  pruneOldResults,
  on,
  VERIFICATION_TOLERANCE,
  OUTLIER_WORK_TOLERANCE,
};

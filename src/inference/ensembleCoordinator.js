"use strict";

/**
 * BTCPC Ensemble Coordinator
 * Shin Devlin
 *
 * Mode A inference sharding: multiple nodes each run the full prompt on their
 * own model. When >= min_ensemble_size nodes return the same result_hash,
 * consensus is declared and all consensus nodes share the reward at a bonus
 * multiplier. Non-consensus contributors get a reduced multiplier.
 *
 * All state is in-memory (no Mongo).
 */

// Bonus multipliers for ensemble work
const ENSEMBLE_BONUS_MULTIPLIER = 1.5;   // consensus nodes earn more
const ENSEMBLE_PARTIAL_MULTIPLIER = 0.5; // contributed but didn't match consensus

// ── In-memory job store ──
// EnsembleJob shape:
// {
//   request_id:      string,
//   model:           string,
//   prompt_hash:     string,
//   min_size:        number,
//   deadline:        number,       // epoch ms
//   contributions:   Map<nodeId, { result_hash, result_text, tokens, model_hash, submitted_at }>,
//   status:          'pending' | 'consensus' | 'partial_win' | 'expired',
//   consensus_hash:  null | string,
//   consensus_nodes: string[],
// }
const jobs = new Map();

/**
 * Create a new ensemble job.
 * @param {string} requestId
 * @param {string} model
 * @param {string} promptHash
 * @param {number} minSize        — minimum matching nodes to declare consensus
 * @param {number} deadlineMs     — absolute epoch ms deadline
 * @returns {object}              — the created job
 */
function createEnsembleJob(requestId, model, promptHash, minSize, deadlineMs) {
  const job = {
    request_id: requestId,
    model: model,
    prompt_hash: promptHash,
    min_size: minSize || 3,
    deadline: deadlineMs || (Date.now() + 120000),
    contributions: new Map(),
    status: "pending",
    consensus_hash: null,
    consensus_nodes: [],
  };
  jobs.set(requestId, job);
  return job;
}

/**
 * Submit a node's inference contribution for an ensemble job.
 * Checks whether consensus has been reached after this submission.
 *
 * @param {string} requestId
 * @param {string} nodeId
 * @param {string} resultHash    — SHA-256 hex of result_text
 * @param {string} resultText    — raw inference output
 * @param {number} tokens        — tokens generated
 * @param {string} modelHash     — hash of the model used (from Ollama /api/show)
 * @returns {{ status, consensusAchieved, consensusNodes, result }}
 */
function submitContribution(requestId, nodeId, resultHash, resultText, tokens, modelHash) {
  const job = jobs.get(requestId);
  if (!job) {
    return { status: "not_found", consensusAchieved: false, consensusNodes: [], result: null };
  }
  if (job.status !== "pending") {
    return { status: job.status, consensusAchieved: job.status === "consensus", consensusNodes: job.consensus_nodes, result: null };
  }
  if (Date.now() > job.deadline) {
    job.status = "expired";
    return { status: "expired", consensusAchieved: false, consensusNodes: [], result: null };
  }

  // Store / overwrite this node's contribution
  job.contributions.set(nodeId, {
    result_hash: resultHash,
    result_text: resultText,
    tokens: tokens || 0,
    model_hash: modelHash || null,
    submitted_at: Date.now(),
  });

  // Check for consensus: count nodes sharing the same result_hash
  const hashCounts = new Map(); // hash → [nodeId, ...]
  for (const [nid, contrib] of job.contributions) {
    const h = contrib.result_hash;
    if (!hashCounts.has(h)) hashCounts.set(h, []);
    hashCounts.get(h).push(nid);
  }

  // Find the hash with the most votes
  let bestHash = null;
  let bestNodes = [];
  for (const [h, nodes] of hashCounts) {
    if (nodes.length > bestNodes.length) {
      bestHash = h;
      bestNodes = nodes;
    }
  }

  if (bestNodes.length >= job.min_size) {
    job.status = "consensus";
    job.consensus_hash = bestHash;
    job.consensus_nodes = bestNodes;
    const winningContrib = job.contributions.get(bestNodes[0]);
    return {
      status: "consensus",
      consensusAchieved: true,
      consensusNodes: bestNodes,
      result: {
        result_hash: bestHash,
        result_text: winningContrib ? winningContrib.result_text : null,
      },
    };
  }

  return {
    status: "pending",
    consensusAchieved: false,
    consensusNodes: [],
    result: null,
  };
}

/**
 * Get the current state of an ensemble job.
 * @param {string} requestId
 * @returns {object|null}
 */
function getJob(requestId) {
  return jobs.get(requestId) || null;
}

/**
 * Mark all past-deadline pending jobs as expired.
 * Call periodically (e.g. every 30s).
 */
function expireOldJobs() {
  const now = Date.now();
  let expired = 0;
  for (const [id, job] of jobs) {
    if (job.status === "pending" && now > job.deadline) {
      job.status = "expired";
      expired++;
    }
  }
  return expired;
}

/**
 * Compute the work value for a single ensemble node.
 *
 * Consensus nodes: tokens × paramCount × ENSEMBLE_BONUS_MULTIPLIER
 * Non-consensus contributors: tokens × paramCount × ENSEMBLE_PARTIAL_MULTIPLIER
 *
 * @param {string} requestId
 * @param {string} nodeId
 * @param {number} paramCount    — verified parameter count from Ollama
 * @returns {number}             — work value (0 if node didn't contribute)
 */
function computeEnsembleWorkValue(requestId, nodeId, paramCount) {
  const job = jobs.get(requestId);
  if (!job) return 0;

  const contrib = job.contributions.get(nodeId);
  if (!contrib) return 0;

  const tokens = contrib.tokens || 0;
  const isConsensus = job.consensus_nodes.includes(nodeId);

  const multiplier = isConsensus ? ENSEMBLE_BONUS_MULTIPLIER : ENSEMBLE_PARTIAL_MULTIPLIER;
  return tokens * paramCount * multiplier;
}

module.exports = {
  ENSEMBLE_BONUS_MULTIPLIER,
  ENSEMBLE_PARTIAL_MULTIPLIER,
  createEnsembleJob,
  submitContribution,
  getJob,
  expireOldJobs,
  computeEnsembleWorkValue,
  // Exposed for testing
  _jobs: jobs,
};

"use strict";

/**
 * HONE Scientific Compute Engine
 * Shin Devlin
 *
 * Long-running distributed scientific compute jobs — protein folding,
 * drug discovery, climate modeling, genomics, and general inference
 * workloads too large or too slow for the real-time inference pipeline.
 *
 * Key mechanics:
 *   - Jobs are queued, assigned to shard groups, and completed
 *   - Open-source jobs receive a 40% fee discount; results go on chain
 *   - Nodes earn 25% bonus for processing open-science jobs
 *   - Results > 50KB are stored in HONE-FS; on-chain entry holds the CID
 *   - Latency map tracks inter-peer round-trips to optimize shard routing
 */

const crypto = require("crypto");

// ─────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────

const OPEN_SOURCE_DISCOUNT = 0.40;    // 40% fee discount for open science jobs
const SCIENCE_NODE_MULTIPLIER = 1.25; // nodes earn 25% more for open science jobs
const MAX_INLINE_RESULT_BYTES = 51200; // 50KB — store inline; larger → HONE-FS blob
const LATENCY_HISTORY_MS = 300000;    // 5 min rolling window for latency scores

// ─────────────────────────────────────────────────────────────────
// In-memory stores
// ─────────────────────────────────────────────────────────────────

/** @type {Map<string, object>} jobId → ScientificJob */
const jobStore = new Map();

/**
 * latencyMap: nodeId → Map<nodeId, { latencyMs, recordedAt }>
 * @type {Map<string, Map<string, { latencyMs: number, recordedAt: number }>>}
 */
const latencyMap = new Map();

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

function generateJobId() {
  return crypto.randomBytes(16).toString("hex");
}

function hashInput(inputData) {
  return crypto.createHash("sha256").update(inputData || "").digest("hex");
}

// ─────────────────────────────────────────────────────────────────
// Fee / payout math
// ─────────────────────────────────────────────────────────────────

/**
 * Compute the actual fee after applying the open-source discount.
 * @param {number} baseFee
 * @param {boolean} openSource
 * @returns {number}
 */
function computeActualFee(baseFee, openSource) {
  if (!baseFee || baseFee <= 0) return 0;
  if (openSource) {
    return parseFloat((baseFee * (1 - OPEN_SOURCE_DISCOUNT)).toFixed(10));
  }
  return baseFee;
}

/**
 * Compute a specific node's payout for a completed job.
 * Open-science jobs apply SCIENCE_NODE_MULTIPLIER to work value.
 * @param {string} jobId
 * @param {string} nodeId
 * @returns {number}
 */
function computeNodePayout(jobId, nodeId) {
  var job = jobStore.get(jobId);
  if (!job) return 0;
  var workValue = (job.work_values && job.work_values[nodeId]) || 0;
  if (job.open_source) {
    return parseFloat((workValue * SCIENCE_NODE_MULTIPLIER).toFixed(10));
  }
  return workValue;
}

// ─────────────────────────────────────────────────────────────────
// Job lifecycle
// ─────────────────────────────────────────────────────────────────

/**
 * Create and store a new scientific compute job.
 *
 * @param {object} params
 * @param {string} params.title
 * @param {string} params.type           — 'protein_folding' | 'drug_discovery' | 'climate' | 'genomics' | 'general'
 * @param {string} params.model          — Ollama model or shard group model
 * @param {string} params.input_data     — base64-encoded input
 * @param {string} params.requester      — HONE account name
 * @param {number} params.max_fee        — HONE
 * @param {boolean} params.open_source   — 40% discount, results stored on chain
 * @returns {object} ScientificJob
 */
function createJob(params) {
  if (!params || !params.requester) throw new Error("requester required");
  if (!params.input_data) throw new Error("input_data required");

  var jobId = generateJobId();
  var openSource = !!params.open_source;
  var baseFee = params.max_fee || 0;
  var feePaid = computeActualFee(baseFee, openSource);

  var job = {
    job_id: jobId,
    title: params.title || jobId,
    type: params.type || "general",
    model: params.model || null,
    input_hash: hashInput(params.input_data),
    input_data: params.input_data,
    requester: params.requester,
    max_fee: baseFee,
    open_source: openSource,
    latency_class: "scientific",
    shard_group_id: null,
    status: "queued",
    result_hash: null,
    result_data: null,
    result_blob_cid: null,
    on_chain: false,
    contributing_nodes: [],
    work_values: {},
    created_at: Date.now(),
    started_at: null,
    completed_at: null,
    fee_paid: feePaid,
    epoch: null,
  };

  jobStore.set(jobId, job);
  return job;
}

/**
 * Mark a job as running and assign it to a shard group.
 * @param {string} jobId
 * @param {string} shardGroupId
 * @returns {object} updated job
 */
function startJob(jobId, shardGroupId) {
  var job = jobStore.get(jobId);
  if (!job) throw new Error("job not found: " + jobId);
  if (job.status !== "queued") throw new Error("job is not queued: " + job.status);

  job.status = "running";
  job.shard_group_id = shardGroupId || null;
  job.started_at = Date.now();
  return job;
}

/**
 * Mark a job complete, store large results in HONE-FS, and if open_source,
 * inscribe a SCIENTIFIC_RESULT ledger entry on chain.
 *
 * @param {string}   jobId
 * @param {string}   resultHash           — SHA-256 hex of raw result
 * @param {string}   resultData           — base64-encoded result bytes
 * @param {string[]} contributingNodes    — nodeIds that did shard work
 * @param {object}   workValues           — nodeId → work_value
 * @param {number}   epoch
 * @returns {{ job, fee_paid, on_chain, result_blob_cid }}
 */
async function completeJob(jobId, resultHash, resultData, contributingNodes, workValues, epoch) {
  var job = jobStore.get(jobId);
  if (!job) throw new Error("job not found: " + jobId);
  if (job.status !== "running") throw new Error("job is not running: " + job.status);

  job.result_hash = resultHash || null;
  job.result_data = resultData || null;
  job.contributing_nodes = Array.isArray(contributingNodes) ? contributingNodes.slice() : [];
  job.work_values = workValues || {};
  job.completed_at = Date.now();
  job.epoch = epoch || null;
  job.status = "complete";

  var resultBlobCid = null;

  // Store large results in HONE-FS
  if (resultData) {
    var rawBytes = Buffer.from(resultData, "base64");
    if (rawBytes.length > MAX_INLINE_RESULT_BYTES) {
      try {
        var blobStore = require("../services/blobStore");
        var stored = blobStore.putBlob(rawBytes);
        resultBlobCid = stored.cid;
        job.result_blob_cid = resultBlobCid;
        // Clear inline data — result lives in HONE-FS
        job.result_data = null;
      } catch (blobErr) {
        // Non-fatal: keep inline data if blob store fails
        console.error("[HONE Science] blob store failed, keeping inline result: " + blobErr.message);
      }
    }
  }

  // Inscribe on-chain if open_source
  var onChain = false;
  if (job.open_source) {
    try {
      var ledger = require("../services/ledger");
      await ledger.recordScientificResult(
        job.job_id,
        job.requester,
        job.model,
        job.input_hash,
        job.result_hash,
        resultBlobCid,
        job.open_source,
        job.fee_paid,
        epoch,
        job.title,
        job.type
      );
      onChain = true;
    } catch (ledgerErr) {
      console.error("[HONE Science] ledger write failed: " + ledgerErr.message);
    }
  }

  job.on_chain = onChain;

  return {
    job: job,
    fee_paid: job.fee_paid,
    on_chain: onChain,
    result_blob_cid: resultBlobCid,
  };
}

/**
 * Retrieve a job by ID.
 * @param {string} jobId
 * @returns {object|null}
 */
function getJob(jobId) {
  return jobStore.get(jobId) || null;
}

/**
 * Return all jobs with status 'queued'.
 * @returns {object[]}
 */
function getOpenJobs() {
  var result = [];
  for (var job of jobStore.values()) {
    if (job.status === "queued") result.push(job);
  }
  return result;
}

/**
 * Return all jobs of a given scientific type.
 * @param {string} type
 * @returns {object[]}
 */
function getJobsByType(type) {
  var result = [];
  for (var job of jobStore.values()) {
    if (job.type === type) result.push(job);
  }
  return result;
}

// ─────────────────────────────────────────────────────────────────
// Latency-aware route optimization
// ─────────────────────────────────────────────────────────────────

/**
 * Record a latency measurement between two nodes.
 * Overwrites any prior measurement for this hop.
 * @param {string} fromNodeId
 * @param {string} toNodeId
 * @param {number} latencyMs
 */
function recordLatency(fromNodeId, toNodeId, latencyMs) {
  if (!fromNodeId || !toNodeId) return;
  if (!latencyMap.has(fromNodeId)) {
    latencyMap.set(fromNodeId, new Map());
  }
  latencyMap.get(fromNodeId).set(toNodeId, {
    latencyMs: latencyMs,
    recordedAt: Date.now(),
  });
}

/**
 * Get the raw latency between two nodes (ignoring window age).
 * Returns null if unknown.
 * @param {string} fromNodeId
 * @param {string} toNodeId
 * @returns {number|null}
 */
function _hopLatency(fromNodeId, toNodeId) {
  var fromMap = latencyMap.get(fromNodeId);
  if (!fromMap) return null;
  var entry = fromMap.get(toNodeId);
  if (!entry) return null;
  // Prune stale entries outside the rolling window
  if (Date.now() - entry.recordedAt > LATENCY_HISTORY_MS) return null;
  return entry.latencyMs;
}

/**
 * Sum latencies for consecutive hops in an ordered node list.
 * Returns Infinity if any hop is unknown.
 * @param {string[]} orderedNodeIds
 * @returns {number}
 */
function getPathLatency(orderedNodeIds) {
  if (!Array.isArray(orderedNodeIds) || orderedNodeIds.length < 2) return 0;
  var total = 0;
  for (var i = 0; i < orderedNodeIds.length - 1; i++) {
    var hop = _hopLatency(orderedNodeIds[i], orderedNodeIds[i + 1]);
    if (hop === null) return Infinity;
    total += hop;
  }
  return total;
}

/**
 * Greedy nearest-neighbor reorder of shard nodes to minimize total path latency.
 * Falls back to original order if latency data is sparse (any hop unknown).
 *
 * @param {object[]} shards  — array of shard node objects, each must have a .nodeId property
 * @returns {object[]}       — reordered shard array
 */
function findOptimalShardOrder(shards) {
  if (!Array.isArray(shards) || shards.length <= 1) return shards;

  // Check if we have enough data to route — any hop between adjacent pairs
  var hasAnyData = false;
  for (var i = 0; i < shards.length - 1; i++) {
    if (_hopLatency(shards[i].nodeId, shards[i + 1].nodeId) !== null) {
      hasAnyData = true;
      break;
    }
  }
  // Also check reverse direction
  if (!hasAnyData) {
    for (var j = 0; j < shards.length - 1; j++) {
      if (_hopLatency(shards[j + 1].nodeId, shards[j].nodeId) !== null) {
        hasAnyData = true;
        break;
      }
    }
  }

  if (!hasAnyData) return shards; // no data at all — fall back

  // Greedy nearest-neighbor starting from the first shard
  var remaining = shards.slice(1);
  var ordered = [shards[0]];

  while (remaining.length > 0) {
    var current = ordered[ordered.length - 1];
    var bestIdx = -1;
    var bestLatency = Infinity;

    for (var k = 0; k < remaining.length; k++) {
      var lat = _hopLatency(current.nodeId, remaining[k].nodeId);
      // If unknown, use a high default so known paths win
      var effective = (lat === null) ? 9999999 : lat;
      if (effective < bestLatency) {
        bestLatency = effective;
        bestIdx = k;
      }
    }

    if (bestIdx === -1) {
      // Fallback: take next in original order
      ordered.push(remaining.shift());
    } else {
      ordered.push(remaining[bestIdx]);
      remaining.splice(bestIdx, 1);
    }
  }

  // Verify the reordering actually improves latency vs original
  var originalLatency = getPathLatency(shards.map(function(s) { return s.nodeId; }));
  var newLatency = getPathLatency(ordered.map(function(s) { return s.nodeId; }));
  if (newLatency < originalLatency) return ordered;
  return shards;
}

// ─────────────────────────────────────────────────────────────────
// Exports
// ─────────────────────────────────────────────────────────────────

module.exports = {
  // Constants
  OPEN_SOURCE_DISCOUNT,
  SCIENCE_NODE_MULTIPLIER,
  MAX_INLINE_RESULT_BYTES,
  LATENCY_HISTORY_MS,
  // Job lifecycle
  createJob,
  startJob,
  completeJob,
  getJob,
  getOpenJobs,
  getJobsByType,
  // Fee / payout
  computeActualFee,
  computeNodePayout,
  // Latency routing
  recordLatency,
  getPathLatency,
  findOptimalShardOrder,
};

"use strict";

/**
 * BTCPC Inference Handler — runs on miner nodes
 * Shin Devlin
 *
 * Listens for INFERENCE_REQUEST messages on the P2P network,
 * auto-claims jobs, runs inference via Ollama, commits/reveals results.
 *
 * This is the miner's side of the inference pipeline.
 */

const crypto = require("crypto");
const axios = require("axios");
const p2p = require("../p2p/network");
const { createMessage } = require("../p2p/protocol");
const { GENESIS_MINER } = require("../mining/genesisBlock");
const { getModelWeight } = require("../mining/workGenerator");
const WorkProof = require("../models/WorkProof");
const Node = require("../models/Node");
const User = require("../models/User");

const OLLAMA_URL = process.env.OLLAMA_URL || "http://100.122.145.60:11434";
const MAX_CONCURRENT = parseInt(process.env.MAX_CONCURRENT_INFERENCE) || 1;

let activeJobs = 0;

// Track request IDs we've already seen to ignore relay echoes
const seenRequests = new Set();
const SEEN_MAX = 1000;

/**
 * Start listening for inference requests on the P2P network.
 * Call this after P2P is connected.
 */
function startInferenceHandler() {
  p2p.onMessage(async (msg, peer) => {
    if (!msg || !msg.type) return;

    switch (msg.type) {
      case "INFERENCE_REQUEST":
        await handleInferenceRequest(msg);
        break;
      case "INFERENCE_ASSIGN":
        await handleAssignment(msg);
        break;
      case "INFERENCE_PAYLOAD":
        await handlePayload(msg);
        break;
      case "MODEL_DEMAND":
        handleModelDemand(msg);
        break;
    }
  });

  console.log("[BTCPC Inference] Handler active, listening for requests");
}

/**
 * Handle incoming inference request — auto-claim if we have capacity.
 */
async function handleInferenceRequest(msg) {
  const data = msg.data || msg;
  const reqId = data.request_id;

  // Deduplicate relay echoes
  if (!reqId || seenRequests.has(reqId)) return;
  seenRequests.add(reqId);
  if (seenRequests.size > SEEN_MAX) {
    const first = seenRequests.values().next().value;
    seenRequests.delete(first);
  }

  if (activeJobs >= MAX_CONCURRENT) {
    console.log(`[BTCPC Inference] Skipping request ${reqId?.slice(0, 8)} — at capacity (${activeJobs}/${MAX_CONCURRENT})`);
    return;
  }

  // Check if we support the requested model
  const model = data.model || "qwen3.5:27b";
  try {
    const modelsResp = await axios.get(`${OLLAMA_URL}/api/tags`, { timeout: 5000 });
    const available = (modelsResp.data.models || []).map(m => m.name);
    if (!available.some(m => m.startsWith(model.split(":")[0]))) {
      console.log(`[BTCPC Inference] Skipping — model ${model} not available`);
      return;
    }
  } catch (_) {
    console.log("[BTCPC Inference] Skipping — Ollama unreachable");
    return;
  }

  // Get our node info for the claim
  const user = await User.findOne({ username: GENESIS_MINER });
  const node = user ? await Node.findOne({ account: user._id }) : null;

  const claim = createMessage("INFERENCE_CLAIM", {
    request_id: data.request_id,
    node_id: node?._id?.toString() || p2p.NODE_ID,
    sik_hash: node?.sik_hash || "none",
    price: Math.min(data.max_fee || 10, 5), // bid 5 or less
    model: model,
    node_name: GENESIS_MINER,
  }, p2p.NODE_ID);

  p2p.broadcast(claim);
  console.log(`[BTCPC Inference] Claimed request ${data.request_id?.slice(0, 8)} at price ${claim.data.price}`);
}

/**
 * Handle assignment — if we're assigned, prepare to receive payload.
 */
async function handleAssignment(msg) {
  const data = msg.data || msg;
  const myNodeId = p2p.NODE_ID;

  const assigned = (data.assignments || []).find(
    a => a.node_id === myNodeId || a.node_name === GENESIS_MINER
  );

  if (!assigned) return; // not assigned to us

  console.log(`[BTCPC Inference] Assigned to request ${data.request_id?.slice(0, 8)}`);
  activeJobs++;
}

/**
 * Handle encrypted payload — decrypt, run inference, commit result.
 * For the initial version, we accept plaintext prompts directly
 * (encryption layer added when SIK is compiled on the node).
 */
async function handlePayload(msg) {
  const data = msg.data || msg;
  const requestId = data.request_id;

  // Deduplicate — only process each payload once
  const payloadKey = 'payload_' + requestId;
  if (!requestId || seenRequests.has(payloadKey)) return;
  seenRequests.add(payloadKey);

  // Check if this payload is for us
  const targetNode = data.node_id || data.target_node;
  if (targetNode && targetNode !== p2p.NODE_ID && targetNode !== GENESIS_MINER) return;
  const prompt = data.prompt; // plaintext for now; encrypted later
  const model = data.model || "qwen3.5:27b";

  if (!prompt) {
    console.log(`[BTCPC Inference] No prompt in payload for ${requestId?.slice(0, 8)}`);
    activeJobs = Math.max(0, activeJobs - 1);
    return;
  }

  console.log(`[BTCPC Inference] Processing request ${requestId?.slice(0, 8)} (${prompt.length} chars, model: ${model})`);

  try {
    // Run inference via Ollama
    const startTime = Date.now();
    const response = await axios.post(`${OLLAMA_URL}/api/generate`, {
      model,
      prompt,
      stream: false,
      options: { temperature: 0.7, num_predict: 1024 },
    }, { timeout: 600000 }); // 10 min — large models on busy GPU need time

    const resultText = response.data.response || "";
    const tokensGenerated = response.data.eval_count || Math.ceil(resultText.length / 4);
    const elapsed = Date.now() - startTime;

    const resultHash = crypto.createHash("sha256").update(resultText).digest("hex");
    const promptHash = crypto.createHash("sha256").update(prompt).digest("hex");

    // Store work proof
    const weightFactor = getModelWeight(model);
    const proof = new WorkProof({
      epoch_number: 0, // will be set by epoch manager
      node_id: GENESIS_MINER,
      prompt_hash: promptHash,
      result_hash: resultHash,
      model,
      tokens_generated: tokensGenerated,
      model_weight_factor: weightFactor,
      work_value: tokensGenerated * weightFactor,
    });
    await proof.save();

    // Commit result hash
    const commit = createMessage("INFERENCE_COMMIT", {
      request_id: requestId,
      node_id: p2p.NODE_ID,
      node_name: GENESIS_MINER,
      result_hash: resultHash,
      tokens_generated: tokensGenerated,
    }, p2p.NODE_ID);
    p2p.broadcast(commit);

    // Immediately reveal (single-node mode; multi-node waits for all commits)
    const reveal = createMessage("INFERENCE_REVEAL", {
      request_id: requestId,
      node_id: p2p.NODE_ID,
      node_name: GENESIS_MINER,
      result_hash: resultHash,
      result_text: resultText, // plaintext for now; encrypted in production
      tokens_generated: tokensGenerated,
      model,
      elapsed_ms: elapsed,
      work_proof: { prompt_hash: promptHash, result_hash: resultHash },
    }, p2p.NODE_ID);
    p2p.broadcast(reveal);

    // Also broadcast as INFERENCE_RESULT for the requester
    const result = createMessage("INFERENCE_RESULT", {
      request_id: requestId,
      result_text: resultText,
      result_hash: resultHash,
      tokens_generated: tokensGenerated,
      model,
      elapsed_ms: elapsed,
      node_name: GENESIS_MINER,
    }, p2p.NODE_ID);
    p2p.broadcast(result);

    console.log(`[BTCPC Inference] Completed ${requestId?.slice(0, 8)}: ${tokensGenerated} tokens, ${elapsed}ms`);
  } catch (err) {
    console.error(`[BTCPC Inference] Failed ${requestId?.slice(0, 8)}:`, err.message);

    // Broadcast failure so requester knows
    const fail = createMessage("INFERENCE_RESULT", {
      request_id: requestId,
      error: err.message,
      node_name: GENESIS_MINER,
    }, p2p.NODE_ID);
    p2p.broadcast(fail);
  } finally {
    activeJobs = Math.max(0, activeJobs - 1);
  }
}

/**
 * Handle MODEL_DEMAND broadcast — log so miners know what to pull.
 */
function handleModelDemand(msg) {
  const data = msg.data || msg;
  console.log(`[BTCPC Inference] \u{1F4E2} MODEL DEMAND: "${data.model}" — ${data.demand} request(s) waiting. Pull this model to earn from unmet demand.`);
}

module.exports = { startInferenceHandler };

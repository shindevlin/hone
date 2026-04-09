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
const MINER_NAME = process.env.BTCPC_MINER || GENESIS_MINER;
const { getModelWeight } = require("../mining/workGenerator");
const WorkProof = require("../models/WorkProof");
const Node = require("../models/Node");
const User = require("../models/User");

const OLLAMA_URL = process.env.OLLAMA_URL || "http://100.122.145.60:11434";
const MAX_CONCURRENT = parseInt(process.env.MAX_CONCURRENT_INFERENCE) || 1;

let activeJobs = 0;
const modelActiveJobs = new Map();
const modelStats = new Map();
let inferenceCount = 0;

// Track request IDs we've already seen to ignore relay echoes
const seenRequests = new Set();
const SEEN_MAX = 1000;
const MODEL_BUSY_THRESHOLD = parseInt(process.env.BTCPC_MODEL_BUSY_THRESHOLD, 10) || 3;

function getModelStats(model) {
  if (!modelStats.has(model)) {
    modelStats.set(model, {
      requests: 0,
      successes: 0,
      failures: 0,
      avgResponseMs: 0,
      avgTokensPerSec: 0
    });
  }
  return modelStats.get(model);
}

function parseModelParams(model) {
  const match = String(model || "").match(/(\d+(?:\.\d+)?)\s*b/i);
  return match ? Number(match[1]) : 0;
}

function promptComplexity(prompt) {
  const text = String(prompt || "");
  if (text.length > 4000 || /analy[sz]e|architecture|proof|derive|implement|refactor|security/i.test(text)) return "high";
  if (text.length > 1200 || /code|debug|explain|compare|summari[sz]e/i.test(text)) return "medium";
  return "low";
}

function modelComplexityFloor(model) {
  const params = parseModelParams(model);
  if (params >= 20) return "high";
  if (params >= 7) return "medium";
  return "low";
}

function complexityRank(value) {
  return { low: 0, medium: 1, high: 2 }[value] || 0;
}

function scoreModel(model, desiredComplexity) {
  const stats = getModelStats(model);
  const successRate = stats.requests ? stats.successes / stats.requests : 1;
  const speed = stats.avgTokensPerSec || 0;
  const busyPenalty = (modelActiveJobs.get(model) || 0) * 100;
  const complexityPenalty = Math.max(0, complexityRank(modelComplexityFloor(model)) - complexityRank(desiredComplexity)) * 2;
  return (parseModelParams(model) * 1000) + (successRate * 100) + speed - busyPenalty - complexityPenalty;
}

function chooseModel(requestedModel, prompt, availableModels) {
  const available = (availableModels || []).filter(Boolean);
  if (!available.length) return requestedModel || "qwen3.5:27b";

  if (requestedModel && requestedModel !== "auto") {
    const exact = available.includes(requestedModel) ? requestedModel : null;
    const withLatest = !requestedModel.includes(":") && available.includes(requestedModel + ":latest") ? requestedModel + ":latest" : null;
    return exact || withLatest || requestedModel;
  }

  const desired = promptComplexity(prompt);
  const eligible = available.filter(model => complexityRank(modelComplexityFloor(model)) >= complexityRank(desired));
  const candidates = eligible.length ? eligible : available;
  const sorted = candidates.slice().sort((a, b) => scoreModel(b, desired) - scoreModel(a, desired));
  const open = sorted.find(model => (modelActiveJobs.get(model) || 0) <= MODEL_BUSY_THRESHOLD);
  return open || sorted[0];
}

function recordModelResult(model, elapsedMs, tokens, success) {
  const stats = getModelStats(model);
  stats.requests++;
  if (success) stats.successes++;
  else stats.failures++;

  if (success) {
    const tokensPerSec = elapsedMs > 0 ? tokens / (elapsedMs / 1000) : 0;
    stats.avgResponseMs = stats.avgResponseMs
      ? (stats.avgResponseMs * 0.85) + (elapsedMs * 0.15)
      : elapsedMs;
    stats.avgTokensPerSec = stats.avgTokensPerSec
      ? (stats.avgTokensPerSec * 0.85) + (tokensPerSec * 0.15)
      : tokensPerSec;
  }

  inferenceCount++;
  if (inferenceCount % 100 === 0) {
    const summary = Array.from(modelStats.entries()).map(([name, s]) => {
      const successRate = s.requests ? ((s.successes / s.requests) * 100).toFixed(1) : "0.0";
      return `${name}: ${s.requests} req, ${successRate}% ok, ${Math.round(s.avgResponseMs)}ms avg, ${s.avgTokensPerSec.toFixed(2)} tok/s`;
    }).join(" | ");
    console.log(`[BTCPC Inference] Model stats after ${inferenceCount} inferences: ${summary}`);
  }
}

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

  // Check if we have the exact requested model
  const requestedModel = data.model || "qwen3.5:27b";
  let model = requestedModel;
  try {
    const modelsResp = await axios.get(`${OLLAMA_URL}/api/tags`, { timeout: 5000 });
    const available = (modelsResp.data.models || []).map(m => m.name);
    model = chooseModel(requestedModel, data.prompt || data.messages?.map(m => m.content).join("\n") || "", available);
    // Exact match: "qwen3:4b" must match "qwen3:4b", not "qwen3.5:27b"
    if (!available.includes(model)) {
      // Also try without tag (e.g. "qwen3:4b" matches "qwen3:4b" but not "qwen3.5:27b")
      const modelBase = model.includes(':') ? model : model + ':latest';
      if (!available.includes(modelBase)) {
        return; // silently skip — don't log spam for every model we don't have
      }
    }
  } catch (_) {
    return; // Ollama unreachable, skip silently
  }

  // Get our node info for the claim
  const user = await User.findOne({ username: MINER_NAME });
  const node = user ? await Node.findOne({ account: user._id }) : null;

  const claim = createMessage("INFERENCE_CLAIM", {
    request_id: data.request_id,
    node_id: node?._id?.toString() || p2p.NODE_ID,
    sik_hash: node?.sik_hash || "none",
    price: Math.min(data.max_fee || 10, 5), // bid 5 or less
    model: model,
    node_name: MINER_NAME,
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
    a => a.node_id === myNodeId || a.node_name === MINER_NAME
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
  if (targetNode && targetNode !== p2p.NODE_ID && targetNode !== MINER_NAME) return;
  const prompt = data.prompt; // plaintext for now; encrypted later
  const model = data.model || "qwen3.5:27b";
  const maxTokens = data.max_tokens || 1024;
  const temperature = data.temperature || 0.7;

  if (!prompt) {
    console.log(`[BTCPC Inference] No prompt in payload for ${requestId?.slice(0, 8)}`);
    activeJobs = Math.max(0, activeJobs - 1);
    return;
  }

  console.log(`[BTCPC Inference] Processing request ${requestId?.slice(0, 8)} (${prompt.length} chars, model: ${model})`);

  try {
    // Run inference via Ollama chat endpoint (works with both chat and completion models)
    const startTime = Date.now();
    modelActiveJobs.set(model, (modelActiveJobs.get(model) || 0) + 1);

    // Parse prompt back into messages if it contains System: prefix, otherwise use as user message
    const messages = [];
    const systemMatch = prompt.match(/^System:\s*([\s\S]*?)(?=\n\n|$)/);
    if (systemMatch) {
      messages.push({ role: 'system', content: systemMatch[1].trim() });
      const userContent = prompt.slice(systemMatch[0].length).trim();
      if (userContent) messages.push({ role: 'user', content: userContent });
    } else {
      messages.push({ role: 'user', content: prompt });
    }

    const response = await axios.post(`${OLLAMA_URL}/api/chat`, {
      model,
      messages,
      stream: false,
      think: false, // disable thinking/reasoning mode — we want content directly
      options: { temperature, num_predict: maxTokens },
    }, { timeout: 600000 }); // 10 min — large models on busy GPU need time

    // Some models (qwen3.5) put output in 'thinking' field when reasoning mode is on
    const msg = response.data.message || {};
    let resultText = msg.content || msg.thinking || response.data.response || "";
    // Strip thinking leaks — qwen3 sometimes exposes reasoning even with think:false
    resultText = resultText.replace(/^(Okay, the user|Hmm,|Interesting|First, I need to|Let me unpack|Let me think)[^\n]*\n+/i, '').trim();
    const tokensGenerated = response.data.eval_count || Math.ceil(resultText.length / 4);
    const elapsed = Date.now() - startTime;

    const resultHash = crypto.createHash("sha256").update(resultText).digest("hex");
    const promptHash = crypto.createHash("sha256").update(prompt).digest("hex");

    // Store InferenceJob locally (authority needs this for settlement sweep)
    const InferenceJob = require("../models/InferenceJob");
    const existingJob = await InferenceJob.findOne({ job_id: requestId });
    if (!existingJob) {
      await InferenceJob.create({
        job_id: requestId,
        status: "completed",
        model,
        messages: [],
        result_text: resultText,
        result_hash: resultHash,
        tokens_generated: tokensGenerated,
        elapsed_ms: elapsed,
        node_name: MINER_NAME,
        completed_at: new Date()
      });
    } else {
      existingJob.status = "completed";
      existingJob.result_text = resultText;
      existingJob.result_hash = resultHash;
      existingJob.tokens_generated = tokensGenerated;
      existingJob.elapsed_ms = elapsed;
      existingJob.node_name = MINER_NAME;
      existingJob.completed_at = new Date();
      await existingJob.save();
    }

    // Store work proof
    const weightFactor = getModelWeight(model);
    const proof = new WorkProof({
      epoch_number: 0, // will be set by epoch manager
      node_id: MINER_NAME,
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
      node_name: MINER_NAME,
      result_hash: resultHash,
      tokens_generated: tokensGenerated,
    }, p2p.NODE_ID);
    p2p.broadcast(commit);

    // Immediately reveal (single-node mode; multi-node waits for all commits)
    const reveal = createMessage("INFERENCE_REVEAL", {
      request_id: requestId,
      node_id: p2p.NODE_ID,
      node_name: MINER_NAME,
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
      node_name: MINER_NAME,
    }, p2p.NODE_ID);
    p2p.broadcast(result);

    // Broadcast VERIFY_REQUEST — verifiers see full response but NOT the prompt
    try {
      const Epoch = require("../models/Epoch");
      const latestEpoch = await Epoch.findOne().sort({ epoch_number: -1 });
      const currentEpoch = latestEpoch ? latestEpoch.epoch_number : 0;
      const verifyReq = createMessage("VERIFY_REQUEST", {
        job_id: requestId,
        result: resultText,
        model,
        token_count: tokensGenerated,
        timing_ms: elapsed,
        miner: MINER_NAME,
        epoch: currentEpoch,
        block_hash: latestEpoch ? (latestEpoch.consensus_hash || "0".repeat(64)) : "0".repeat(64)
      }, p2p.NODE_ID);
      p2p.broadcast(verifyReq);
    } catch (verifyErr) {
      console.error(`[BTCPC Inference] Failed to broadcast VERIFY_REQUEST: ${verifyErr.message}`);
    }

    console.log(`[BTCPC Inference] Completed ${requestId?.slice(0, 8)}: ${tokensGenerated} tokens, ${elapsed}ms`);
    recordModelResult(model, elapsed, tokensGenerated, true);
  } catch (err) {
    console.error(`[BTCPC Inference] Failed ${requestId?.slice(0, 8)}:`, err.message);
    recordModelResult(model, 0, 0, false);

    // Broadcast failure so requester knows
    const fail = createMessage("INFERENCE_RESULT", {
      request_id: requestId,
      error: err.message,
      node_name: MINER_NAME,
    }, p2p.NODE_ID);
    p2p.broadcast(fail);
  } finally {
    activeJobs = Math.max(0, activeJobs - 1);
    modelActiveJobs.set(model, Math.max(0, (modelActiveJobs.get(model) || 0) - 1));
  }
}

/**
 * Handle MODEL_DEMAND broadcast — log so miners know what to pull.
 */
function handleModelDemand(msg) {
  const data = msg.data || msg;
  console.log(`[BTCPC Inference] \u{1F4E2} MODEL DEMAND: "${data.model}" — ${data.demand} request(s) waiting. Pull this model to earn from unmet demand.`);
}

module.exports = {
  startInferenceHandler,
  _modelRouting: {
    chooseModel,
    getModelStats,
    modelStats,
    modelActiveJobs,
    parseModelParams,
    promptComplexity,
    recordModelResult
  }
};
